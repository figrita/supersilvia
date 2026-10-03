// SPDX-License-Identifier: AGPL-3.0-or-later

//! Video on Windows: Media Foundation for a camera, and each GPU vendor's own hardware codecs,
//! with Direct3D 12's after them, all through GStreamer's official release.
//!
//! **A camera is saved by its device path and opened by it.** The device monitor over
//! `Video/Source` answers through `mfdeviceprovider`, which opens nothing, and each device
//! carries `device.path`, the symbolic link Windows keeps for the device across reboots, which
//! `mfvideosrc` takes as `device-path`. A menu shows the name alone, since the path is a string
//! a person never reads. Only Media Foundation's devices are listed: DirectShow's and kernel
//! streaming's providers list the same cameras again, and the screen-capture providers list
//! monitors under the same class.
//!
//! **The codecs are each vendor's, then Direct3D 12's.** GStreamer registers NVENC's, Quick
//! Sync's and AMF's elements only on a machine with that vendor's GPU and driver, and
//! Direct3D 12's encoder only where a device offers video encoding, so whichever comes first in
//! [`CODECS`] is the one this machine has. NVENC's rows carry Linux's names and Linux's
//! properties, so an entry one machine writes plays on the other. AMF has no decoder of its
//! own, so its rows decode with Direct3D 11's, as Quick Sync's AV1 does. Every row asks for a
//! constant QP of 26, which is VA-API's default and x264's.
//!
//! **No zero-copy.** A decoder's frames reach the GPU through memory, as on a machine with no
//! DMA-BUF import: the renderer's device is Vulkan, and sharing a Direct3D texture with it is
//! not written.

use crate::nodes::Frame;
use crate::video::clip::Codec;
use gstreamer as gst;
use gstreamer::prelude::*;

// ------------------------------------------------------------------------------ cameras

/// A camera is saved by a path a person never reads, so a menu shows its name alone.
pub const NAMED_CAMERAS: bool = true;

/// Every camera Media Foundation lists, as `(path, name, caps)`, in its order.
fn listed() -> Vec<(String, String, Option<gst::Caps>)> {
    if gst::init().is_err() {
        return Vec::new();
    }
    // Ours before any monitor loads a system's NDI plugin in its place: see `video::ndi`.
    let _ = crate::video::ndi::register();
    let monitor = gst::DeviceMonitor::new();
    monitor.add_filter(Some("Video/Source"), None);
    if monitor.start().is_err() {
        return Vec::new();
    }
    let found = monitor
        .devices()
        .iter()
        .filter_map(|d| {
            let props = d.properties()?;
            if props.get::<String>("device.api").ok()? != "mediafoundation" {
                return None;
            }
            let path: String = props.get("device.path").ok()?;
            Some((path, d.display_name().to_string(), d.caps()))
        })
        .collect();
    monitor.stop();
    found
}

/// Every camera Media Foundation lists, as `(path, name)`, in its order.
pub fn capture_devices() -> Vec<(String, String)> {
    listed()
        .into_iter()
        .map(|(path, name, _)| (path, name))
        .collect()
}

/// The formats and sizes the camera at `path` offers, from the device monitor's listing, which
/// opens no camera. `None` where it is not listed.
pub fn device_caps(path: &str) -> Option<gst::Caps> {
    listed()
        .into_iter()
        .find(|(listed, ..)| listed == path)
        .and_then(|(.., caps)| caps)
}

/// The first camera Media Foundation lists.
pub fn first_capture_device() -> Option<String> {
    listed().into_iter().next().map(|(path, ..)| path)
}

/// The source element for the camera at this device path.
pub fn device_element(path: &str) -> Result<String, String> {
    Ok(format!("mfvideosrc device-path={}", super::quoted(path)))
}

/// Nothing: decodebin's own order is kept for a webcam's JPEG.
pub fn prefer_hardware_jpeg(_decodebin: &gst::Element) {}

// ------------------------------------------------------------------------------- codecs

/// The hardware codecs a clip's cache may be written with, in order of preference: NVIDIA's,
/// Intel's, AMD's, then Direct3D 12's, each vendor's H.264 first.
pub const CODECS: &[Codec] = &[
    Codec {
        name: "nv-h264",
        encoder: "nvh264enc",
        decoder: "nvh264dec",
        parser: "h264parse",
        intra: "gop-size=1 rc-mode=constqp",
        delivery: "rc-mode=constqp",
    },
    Codec {
        name: "nv-h265",
        encoder: "nvh265enc",
        decoder: "nvh265dec",
        parser: "h265parse",
        intra: "gop-size=1 rc-mode=constqp",
        delivery: "rc-mode=constqp",
    },
    Codec {
        name: "nv-av1",
        encoder: "nvav1enc",
        decoder: "nvav1dec",
        parser: "av1parse",
        intra: "gop-size=1 rc-mode=constqp",
        delivery: "rc-mode=constqp",
    },
    Codec {
        name: "qsv-h264",
        encoder: "qsvh264enc",
        decoder: "qsvh264dec",
        parser: "h264parse",
        intra: "gop-size=1 rate-control=cqp qp-i=26 qp-p=26",
        delivery: "rate-control=cqp qp-i=26 qp-p=26",
    },
    Codec {
        name: "qsv-h265",
        encoder: "qsvh265enc",
        decoder: "qsvh265dec",
        parser: "h265parse",
        intra: "gop-size=1 rate-control=cqp qp-i=26 qp-p=26",
        delivery: "rate-control=cqp qp-i=26 qp-p=26",
    },
    Codec {
        name: "qsv-av1",
        encoder: "qsvav1enc",
        decoder: "d3d11av1dec",
        parser: "av1parse",
        intra: "gop-size=1 rate-control=cqp qp-i=26 qp-p=26",
        delivery: "rate-control=cqp qp-i=26 qp-p=26",
    },
    Codec {
        name: "amf-h264",
        encoder: "amfh264enc",
        decoder: "d3d11h264dec",
        parser: "h264parse",
        intra: "gop-size=1 rate-control=cqp qp-i=26 qp-p=26",
        delivery: "rate-control=cqp qp-i=26 qp-p=26",
    },
    Codec {
        name: "amf-h265",
        encoder: "amfh265enc",
        decoder: "d3d11h265dec",
        parser: "h265parse",
        intra: "gop-size=1 rate-control=cqp qp-i=26 qp-p=26",
        delivery: "rate-control=cqp qp-i=26 qp-p=26",
    },
    Codec {
        name: "amf-av1",
        encoder: "amfav1enc",
        decoder: "d3d11av1dec",
        parser: "av1parse",
        intra: "gop-size=1 rate-control=cqp qp-i=26 qp-p=26",
        delivery: "rate-control=cqp qp-i=26 qp-p=26",
    },
    Codec {
        name: "d3d12-h264",
        encoder: "d3d12h264enc",
        decoder: "d3d12h264dec",
        parser: "h264parse",
        intra: "gop-size=1 rate-control=cqp qp-i=26 qp-p=26",
        delivery: "rate-control=cqp qp-i=26 qp-p=26",
    },
];

/// What a status line names when [`CODECS`] has nothing this machine can run.
pub const CODEC_HINT: &str = "NVENC, Quick Sync, AMF or Direct3D 12 video encoding";

/// Nothing: these codecs drain every frame they hold at the end of a stream, as Linux's do.
pub fn settle_before_eos(_pipeline: &gst::Pipeline) {}

// ----------------------------------------------------------------------------- zero-copy

/// `None`: no DMA-BUF is exported here.
pub fn dmabuf_format() -> Option<String> {
    None
}

/// False: a decoded clip reaches the GPU through memory.
pub fn clip_dmabuf() -> bool {
    false
}

/// False: the renderer imports no frame without a copy here.
pub fn dmabuf_imports() -> bool {
    false
}

/// `None`: there is no zero-copy chain.
pub fn dmabuf_chain() -> Option<String> {
    None
}

/// `None`: a screen capture delivers bytes.
pub fn dmabuf_caps() -> Option<gst::Caps> {
    None
}

/// `None`: every sample is mapped as bytes.
pub fn dmabuf_frame(
    _caps: &gst::CapsRef,
    _buffer: &gst::BufferRef,
) -> Option<Result<Frame, String>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only prints. Whether a camera exists is a fact about the machine, not the code.
    #[test]
    fn capture_devices_can_be_listed() {
        for (path, name) in capture_devices() {
            println!("{path}: {name}");
        }
        println!("first capture device: {:?}", first_capture_device());
    }

    /// Every row's encoder parses with its properties, where this machine has it.
    #[test]
    fn every_installed_encoder_takes_its_properties() {
        gst::init().expect("gstreamer");
        for codec in CODECS {
            if gst::ElementFactory::find(codec.encoder).is_none() {
                continue;
            }
            for properties in [codec.intra, codec.delivery] {
                let chain = format!("{} {properties}", codec.encoder);
                gst::parse::launch(&chain).unwrap_or_else(|e| panic!("{chain}: {e}"));
            }
        }
    }
}
