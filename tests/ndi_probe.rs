// SPDX-License-Identifier: AGPL-3.0-or-later

//! **The NDI® runtime probe puts nothing on the network.** Every launch asks whether the runtime
//! loads (`video::ndi::start`), and the answer must come without an NDI sender, which announces
//! itself to the local network: supersilvia uses the network only for NDI someone is using
//! (`docs/decisions.md`). The plugin's `ndisink` logs *Started* when it makes one, so this
//! listens to GStreamer's log for that while the probe runs, then makes one on purpose to show
//! the listening hears it.
//!
//! A file of its own, because `tests/ndi.rs` sends over NDI in the same process. Without the
//! runtime it skips, saying so, as `tests/ndi.rs` does.

use gstreamer as gst;
use gstreamer::prelude::*;
use std::sync::{Arc, Mutex};

#[test]
fn the_runtime_probe_starts_no_ndi_sender() {
    gst::init().unwrap();
    gst::log::remove_default_log_function();
    gst::log::set_threshold_for_name("ndisink", gst::DebugLevel::Info);
    let heard: Arc<Mutex<Vec<String>>> = Arc::default();
    let into = Arc::clone(&heard);
    gst::log::add_log_function(move |category, _, _, _, _, _, message| {
        if category.name() == "ndisink"
            && let Some(text) = message.get()
        {
            into.lock().unwrap().push(text.to_string());
        }
    });

    if !supersilvia::video::ndi::runtime() {
        eprintln!("no NDI runtime here; skipping");
        return;
    }
    let during = heard.lock().unwrap().clone();
    assert!(
        !during.iter().any(|m| m.contains("Started")),
        "the probe started an NDI sender: {during:?}"
    );

    // The same listening, around a sender made on purpose.
    let sink = gst::ElementFactory::make("ndisink").build().unwrap();
    let pipeline = gst::Pipeline::new();
    pipeline.add(&sink).unwrap();
    pipeline.set_state(gst::State::Ready).unwrap();
    pipeline.set_state(gst::State::Null).unwrap();
    assert!(
        heard.lock().unwrap().iter().any(|m| m.contains("Started")),
        "an ndisink taken to Ready was not heard starting: {:?}",
        heard.lock().unwrap()
    );
}
