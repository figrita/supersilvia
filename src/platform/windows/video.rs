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
//! **A clip's frame reaches the GPU without a copy**, as Direct3D 12 textures on the
//! renderer's adapter. The zero-copy chain ends in `videoconvert ! d3d12upload` and caps in
//! `memory:D3D12Memory`, so a Direct3D 12 decoder's own textures go through both untouched, and
//! any other decoder's frames are converted to a format the renderer samples and uploaded. The
//! conversion is `videoconvert`'s on the CPU rather than `d3d12convert`'s, which compiles its
//! shaders when it starts and so fails where the system's shader compiler cannot, as Wine's;
//! a hardware decoder's frames need neither. [`dmabuf_context`] hands the pipeline the
//! renderer's device, and [`dmabuf_frame`] finds the texture, its slice and its fence behind the
//! sample (`super::d3d12`). Whether the renderer imports one is the renderer's to say, so
//! [`dmabuf_imports`] asks it — the edge `tests/rules.rs` names, as Linux's and the Mac's are.
//! A screen delivers bytes, so [`dmabuf_caps`] and [`dmabuf_format`] have nothing to say.

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

/// The element the zero-copy chain uploads with, where a frame is not in a texture already.
const UPLOAD: &str = "d3d12upload";

/// `None`: no DMA-BUF is exported here.
pub fn dmabuf_format() -> Option<String> {
    None
}

/// Can a decoded clip reach the GPU without a copy here: the renderer's device imports a
/// Direct3D 12 texture, GStreamer makes a device on its adapter, and the chain's upload is
/// installed.
pub fn clip_dmabuf() -> bool {
    dmabuf_imports()
        && super::d3d12::context().is_some()
        && gst::ElementFactory::find(UPLOAD).is_some()
}

/// Can the renderer's device import a Direct3D 12 texture. False before there is a renderer.
pub fn dmabuf_imports() -> bool {
    crate::render::dmabuf::imports()
}

/// The conversion that hands a decoder's or a source's frames to the sink as Direct3D 12
/// textures, in a format the renderer samples: NV12 where its device takes NV12, and RGB.
pub fn dmabuf_chain() -> Option<String> {
    let here = crate::render::dmabuf::d3d12_here()?;
    let formats = if here.nv12 {
        "{NV12,RGBA,BGRA}"
    } else {
        "{RGBA,BGRA}"
    };
    Some(format!(
        "videoconvert ! video/x-raw(ANY),format={formats} ! {UPLOAD} \
         ! video/x-raw({}),format={formats}",
        super::d3d12::MEMORY
    ))
}

/// Hand a zero-copy pipeline the renderer's device, so its Direct3D 12 elements make their
/// textures where the renderer samples them.
pub fn dmabuf_context(pipeline: &gst::Pipeline) {
    super::d3d12::hand_device(pipeline);
}

/// `None`: a screen capture delivers bytes.
pub fn dmabuf_caps() -> Option<gst::Caps> {
    None
}

/// A sample in Direct3D 12 memory as a `Frame` the renderer imports, or as its bytes where it
/// cannot; `None` for a sample in memory, which the caller maps as bytes.
pub fn dmabuf_frame(caps: &gst::CapsRef, buffer: &gst::BufferRef) -> Option<Result<Frame, String>> {
    super::d3d12::frame(caps, buffer)
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
