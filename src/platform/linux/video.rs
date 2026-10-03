// SPDX-License-Identifier: AGPL-3.0-or-later

//! Video on Linux: V4L2 for a camera, VA-API or NVENC for the hardware codecs, and DMA-BUF
//! for a frame that reaches the GPU without a copy.
//!
//! **Whether the GPU can import a DMA-BUF is the renderer's to say.** The question needs the
//! renderer's device, which only `render/dmabuf.rs` is served, so [`dmabuf_imports`] and
//! [`dmabuf_caps`] ask it — the one edge out of the pure modules' reach that
//! `tests/rules.rs` steps over, named there by both files.

use crate::nodes::{DmaBuf, Frame, Pixels};
use crate::video::clip::Codec;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_allocators::DmaBufMemory;
use gstreamer_video as gst_video;
use std::sync::Arc;

// ------------------------------------------------------------------------------ cameras

/// A camera is saved by its device path, which a menu shows as it is.
pub const NAMED_CAMERAS: bool = false;

/// Every V4L2 capture device GStreamer can see, as `(path, name)`, in probe order.
pub fn capture_devices() -> Vec<(String, String)> {
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
            let path = v4l2_path(&props)?;
            let name = props
                .get::<String>("api.v4l2.cap.card")
                .unwrap_or_else(|_| d.display_name().to_string());
            Some((path, name))
        })
        .collect();
    monitor.stop();
    found
}

/// A listed device's V4L2 node. The V4L2 provider calls it `device.path`; PipeWire's calls it
/// `api.v4l2.path`, and where both are installed GStreamer lists PipeWire's and hides the
/// other. PipeWire's also carries the card's own name as `api.v4l2.cap.card`, which is the
/// name the V4L2 provider would have listed, where its display name is only the product ID.
fn v4l2_path(props: &gst::StructureRef) -> Option<String> {
    props
        .get::<String>("device.path")
        .or_else(|_| props.get::<String>("api.v4l2.path"))
        .ok()
}

/// The formats and sizes the V4L2 device at `path` offers, raw and JPEG, from the device
/// monitor's listing. `None` where the monitor does not list it, which it does not where udev
/// is out of reach.
pub fn device_caps(path: &str) -> Option<gst::Caps> {
    gst::init().ok()?;
    // Ours before any monitor loads a system's NDI plugin in its place: see `video::ndi`.
    let _ = crate::video::ndi::register();
    let monitor = gst::DeviceMonitor::new();
    monitor.add_filter(Some("Video/Source"), None);
    monitor.start().ok()?;
    let caps = monitor.devices().iter().find_map(|d| {
        let props = d.properties()?;
        let listed = v4l2_path(&props)?;
        if listed == path { d.caps() } else { None }
    });
    monitor.stop();
    caps
}

/// The first device that can actually capture.
///
/// The device monitor answers only where udev is reachable, which inside a container it
/// often is not. Then every `/dev/video*` is tried in order: a webcam exposes a metadata
/// node beside its capture node, and a loopback or virtual camera is output-only, so the
/// only reliable question is whether `v4l2src` will open it.
pub fn first_capture_device() -> Option<String> {
    let mut devices = capture_devices();
    // Probe order is not device order; the lowest-numbered node is the least surprising.
    devices.sort();
    if let Some((path, _)) = devices.into_iter().next() {
        return Some(path);
    }
    let mut nodes: Vec<String> = std::fs::read_dir("/dev")
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with("video").then(|| format!("/dev/{name}"))
        })
        .collect();
    nodes.sort_by_key(|p| {
        p.trim_start_matches("/dev/video")
            .parse::<u32>()
            .unwrap_or(u32::MAX)
    });
    nodes.into_iter().find(|p| can_capture(p))
}

/// Does `v4l2src` accept this node as a capture device? Reaching `Ready` is enough to open
/// it and read its capabilities without starting a stream.
fn can_capture(path: &str) -> bool {
    let Ok(src) = gst::ElementFactory::make("v4l2src")
        .property("device", path)
        .build()
    else {
        return false;
    };
    let ok = src.set_state(gst::State::Ready).is_ok();
    let _ = src.set_state(gst::State::Null);
    ok
}

/// The source element for a V4L2 device node such as `/dev/video0`.
pub fn device_element(path: &str) -> Result<String, String> {
    Ok(format!("v4l2src device={path}"))
}

/// Have `decodebin` decode JPEG with `vajpegdec` where the element exists.
///
/// An MJPEG webcam's every frame is a JPEG, and `jpegdec` decodes it on one CPU thread.
/// `vajpegdec` decodes on the video engine, but its rank is none, so decodebin never
/// considers it. Raising the rank would reach every decodebin in the process — a poster
/// of a progressive JPEG the video engine cannot decode among them — so the preference is
/// this pipeline's alone: once `jpegparse` has said what the stream is, the decoder list
/// decodebin is about to try gets `vajpegdec` at its head, and `jpegdec` stays behind it
/// for a stream it refuses.
pub fn prefer_hardware_jpeg(decodebin: &gst::Element) {
    let Some(va) = gst::ElementFactory::find("vajpegdec") else {
        return;
    };
    decodebin.connect("autoplug-sort", false, move |args| {
        // Null is "no opinion": decodebin keeps its own order.
        let none = None::<gst::glib::ValueArray>.to_value();
        let caps = args.get(2).and_then(|v| v.get::<gst::Caps>().ok());
        let factories = args
            .get(3)
            .and_then(|v| v.get::<gst::glib::ValueArray>().ok());
        let (Some(caps), Some(factories)) = (caps, factories) else {
            return Some(none);
        };
        if !caps.iter().any(|s| s.name() == "image/jpeg") || !va.can_sink_all_caps(&caps) {
            return Some(none);
        }
        let mut order = gst::glib::ValueArray::new([va.to_value()]);
        for f in factories.iter() {
            if f.get::<gst::ElementFactory>().ok().as_ref() != Some(&va) {
                order.append(f.clone());
            }
        }
        Some(order.to_value())
    });
}

// ------------------------------------------------------------------------------- codecs

/// The hardware codecs a clip's cache may be written with, in order of preference. The VA
/// elements come first because that is what Mesa exposes on AMD and Intel; the `nv` ones are
/// NVIDIA's own plugin.
pub const CODECS: &[Codec] = &[
    Codec {
        name: "h264",
        encoder: "vah264enc",
        decoder: "vah264dec",
        parser: "h264parse",
        intra: "key-int-max=1 rate-control=cqp",
        delivery: "rate-control=cqp",
    },
    Codec {
        name: "h265",
        encoder: "vah265enc",
        decoder: "vah265dec",
        parser: "h265parse",
        intra: "key-int-max=1 rate-control=cqp",
        delivery: "rate-control=cqp",
    },
    Codec {
        name: "av1",
        encoder: "vaav1enc",
        decoder: "vaav1dec",
        parser: "av1parse",
        intra: "key-int-max=1 rate-control=cqp",
        delivery: "rate-control=cqp",
    },
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
];

/// What a status line names when [`CODECS`] has nothing this machine can run.
pub const CODEC_HINT: &str = "VA-API (Mesa) or NVENC";

/// Nothing: the VA-API and NVENC codecs drain every frame they hold at the end of a stream.
pub fn settle_before_eos(_pipeline: &gst::Pipeline) {}

// ----------------------------------------------------------------------------- DMA-BUF

/// The DRM format `vapostproc` will export RGBA as on this machine, with the modifier the
/// driver chose, as a caps string — or `None` where the element or the format is missing.
///
/// Read from the element's own pad template, because the modifier is the GPU's tiling and
/// differs by vendor and generation; asking for a linear layout would make the video engine
/// untile every frame.
pub fn dmabuf_format() -> Option<String> {
    gst::init().ok()?;
    let factory = gst::ElementFactory::find("vapostproc")?;
    let template = factory
        .static_pad_templates()
        .into_iter()
        .find(|t| t.direction() == gst::PadDirection::Src)?;
    let caps = template.caps();
    for s in caps.iter() {
        if !s.name().starts_with("video/x-raw") {
            continue;
        }
        let Ok(list) = s.get::<gst::List>("drm-format") else {
            continue;
        };
        for v in list.iter() {
            let Ok(f) = v.get::<String>() else { continue };
            // RGBA in memory order first, then the byte-swapped one the device still imports.
            if f.starts_with("AB24:") || f.starts_with("AR24:") {
                return Some(f);
            }
        }
    }
    None
}

/// Can a decoded clip reach the GPU as DMA-BUFs here: `vapostproc` exports a format, and
/// the renderer's device imports one.
pub fn clip_dmabuf() -> bool {
    dmabuf_format().is_some() && dmabuf_imports()
}

/// Can the renderer's device import a DMA-BUF at all. False before there is a renderer.
pub fn dmabuf_imports() -> bool {
    crate::render::dmabuf::imports()
}

/// The conversion that exports a decoder's or a camera's frames as RGBA DMA-BUFs, up to the
/// sink: `vapostproc` on the video engine, in the format [`dmabuf_format`] read. `None`
/// where there is none.
pub fn dmabuf_chain() -> Option<String> {
    Some(format!(
        "vapostproc ! video/x-raw(memory:DMABuf),format=DMA_DRM,drm-format={}",
        dmabuf_format()?
    ))
}

/// Nothing: the frames a zero-copy pipeline makes need no device handed to it here.
pub fn dmabuf_context(_pipeline: &gst::Pipeline) {}

/// The DMA-BUF caps a screen cast's appsink accepts: the RGB formats and tilings the
/// renderer's device imports. `None` where nothing is importable here — there is no renderer
/// yet, or none of its formats is one this reads.
pub fn dmabuf_caps() -> Option<gst::Caps> {
    let formats: Vec<String> = crate::render::dmabuf::importable_here()
        .into_iter()
        .map(|(fourcc, modifier)| gst_video::dma_drm_fourcc_to_string(fourcc, modifier).into())
        .collect();
    if formats.is_empty() {
        return None;
    }
    Some(
        gst::Caps::builder("video/x-raw")
            .features([gstreamer_allocators::CAPS_FEATURE_MEMORY_DMABUF])
            .field("format", "DMA_DRM")
            .field("drm-format", gst::List::new(formats))
            .build(),
    )
}

/// A sample in DMA-BUF memory as a `Frame`: the descriptor, its format and tiling, and the
/// buffer held behind it so the memory stays valid for as long as the frame does. `None`
/// where the sample's caps are not DMA-BUF caps, and the caller maps it as bytes.
pub fn dmabuf_frame(caps: &gst::CapsRef, buffer: &gst::BufferRef) -> Option<Result<Frame, String>> {
    gst_video::is_dma_drm_caps(caps).then(|| described(caps, buffer))
}

/// The descriptor behind a sample whose caps say DMA-BUF.
fn described(caps: &gst::CapsRef, buffer: &gst::BufferRef) -> Result<Frame, String> {
    let drm = gst_video::VideoInfoDmaDrm::from_caps(caps).map_err(|e| e.to_string())?;
    let mem = buffer.peek_memory(0);
    let dma = mem
        .downcast_memory_ref::<DmaBufMemory>()
        .ok_or("DMA_DRM caps on memory that is not a DMA-BUF")?;
    // The stride and offset travel in the meta, not the caps; that is why the sink has to
    // advertise it.
    let (stride, offset) = if let Some(m) = buffer.meta::<gst_video::VideoMeta>() {
        (m.stride()[0] as u32, m.offset()[0] as u32)
    } else {
        let info = drm.to_video_info().map_err(|e| e.to_string())?;
        (info.stride()[0] as u32, 0)
    };
    Ok(Frame {
        width: drm.width(),
        height: drm.height(),
        pixels: Pixels::DmaBuf(DmaBuf {
            fd: dma.fd(),
            fourcc: drm.fourcc(),
            modifier: drm.modifier(),
            stride,
            offset: offset + mem.offset() as u32,
            keep: Arc::new(buffer.to_owned()),
            refused: None,
        }),
    })
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

    /// Either provider's name for the node is read: V4L2's own, and PipeWire's, which hides
    /// V4L2's where both are installed.
    #[test]
    fn a_device_path_is_read_from_either_provider() {
        gst::init().unwrap();
        let v4l2 = gst::Structure::builder("v4l2deviceprovider")
            .field("device.path", "/dev/video1")
            .build();
        let pipewire = gst::Structure::builder("pipewire-proplist")
            .field("api.v4l2.path", "/dev/video1")
            .field("object.path", "v4l2:/dev/video1")
            .build();
        let neither = gst::Structure::builder("pipewire-proplist").build();
        assert_eq!(v4l2_path(&v4l2).as_deref(), Some("/dev/video1"));
        assert_eq!(v4l2_path(&pipewire).as_deref(), Some("/dev/video1"));
        assert_eq!(v4l2_path(&neither), None);
    }
}
