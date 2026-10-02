// SPDX-License-Identifier: AGPL-3.0-or-later

//! A device `render::gpu::watch`es: an error nothing captured is logged rather than ending the
//! process, and a lost device reaches the callback. The loss is forced with
//! `Device::destroy`, the one wgpu offers, on a device of this test's own.
//!
//! What the app does with the loss — the autosave written at once, and the line it says — is
//! `tests/autosave.rs`'s.

#[path = "common/gpu.rs"]
mod gpu;

use std::time::Duration;

#[test]
fn an_uncaptured_error_is_logged_and_a_lost_device_is_reported() {
    // The harness's device panics on any validation error; watching it replaces that with
    // the app's own handler.
    let gpu = gpu::gpu();
    let (tx, rx) = std::sync::mpsc::channel();
    supersilvia::render::gpu::watch(gpu.device(), move |why| {
        let _ = tx.send(why);
    });

    // Mapped for reading and for writing both, which no device takes without a feature.
    let _refused = gpu.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("invalid"),
        size: 4,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::MAP_WRITE,
        mapped_at_creation: false,
    });
    assert!(
        rx.try_recv().is_err(),
        "a validation error is not a lost device"
    );

    gpu.device().destroy();
    let _ = gpu.device().poll(wgpu::PollType::wait_indefinitely());
    let why = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the lost-device callback ran");
    assert!(why.starts_with("Destroyed"), "{why}");
}
