// SPDX-License-Identifier: AGPL-3.0-or-later

//! Video on macOS: AVFoundation for a camera and VideoToolbox for the hardware codecs, both
//! through GStreamer's `applemedia` plugin.
//!
//! **A camera is saved by AVFoundation's unique ID and opened by its index.** The device
//! monitor over `Video/Source` answers through `avfdeviceprovider` and opens nothing, and each
//! device carries `avf.unique_id` but no `device.path`. `avfvideosrc` takes only
//! `device-index`, which the element a device builds already has set. So an ID is found again
//! by listing and matching, and the index is read off that element without starting it. A
//! menu shows the name alone, since the ID is a string a person never reads.
//! `docs/decisions.md` has why the ID and not the position.
//!
//! **A camera is asked for frames in system memory.** `avfvideosrc` lists every size's UYVY
//! and YUY2 as GL rectangle textures before anything in memory, and fixates on the first
//! structure its peer accepts. With no size asked for, the peer is `decodebin`, which accepts
//! anything, so left to itself the camera settles on a GL texture that no `videoconvert` on
//! the way to the appsink can take, and the pipeline stops *not-linked* before its first
//! frame. The element [`device_element`] answers carries `video/x-raw`, so the first
//! structure left is the first size in memory: for the FaceTime HD camera of an M2 MacBook
//! Air, as the device monitor lists it, 1552x1552 UYVY. A reader asking for no size is given
//! 1920x1080 or 1280x720 by [`crate::video`] where the listing's caps carry one in memory,
//! so that first size is what a camera offering neither opens at.
//!
//! **A webcam's JPEG needs no preference.** `vtdec_hw` (rank 257) already outranks `jpegdec`
//! (256), and `avfvideosrc` offers only raw NV12, UYVY, YUY2, ARGB and BGRA anyway.
//!
//! **A clip's frame reaches the GPU without a copy.** `vtdec_hw`'s plain output is its own
//! `CVPixelBuffer`, NV12 on a two-plane `IOSurface`, and `videoconvert` passes it through
//! untouched, so the zero-copy chain is the bytes chain: [`dmabuf_frame`] finds the surface
//! behind the sample and answers `Pixels::IoSurface`, the same memory mapped beside it, which
//! `render/dmabuf.rs` imports. Whether the renderer's device imports one is the renderer's to
//! say, so [`dmabuf_imports`] asks it — the edge `tests/rules.rs` names, as Linux's is. A
//! screen is no pipeline, so [`dmabuf_caps`] and [`dmabuf_format`] have nothing to say.
//!
//! **The codec rows carry Linux's names.** A cache entry's name carries its codec, so an
//! `h264` entry written by `vah264enc` on Linux plays here with no second import, and one
//! written here plays there. H.264 comes first for the same reason: both machines choose it.
//! VideoToolbox has no constant-QP mode, so the rows ask for a constant `quality`, and 0.7 is
//! where x264's QP 26 lands; `proposals/macos-media.md` has the measurements.

use crate::nodes::{Frame, IoSurface, Pixels};
use crate::video::clip::Codec;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_video as gst_video;
use gstreamer_video::prelude::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

// ------------------------------------------------------------------------------ cameras

/// A camera is saved by an ID a person never reads, so a menu shows its name alone.
pub const NAMED_CAMERAS: bool = true;

/// One camera as AVFoundation lists it.
#[derive(Debug, Clone)]
struct Listed {
    /// `avf.unique_id`, which a project saves.
    id: String,
    name: String,
    /// What `avfvideosrc` opens it at.
    index: i32,
    /// The formats and sizes it offers, as the device monitor lists them.
    caps: Option<gst::Caps>,
}

/// The last listing, which [`device_element`] looks an ID up in before it lists again.
static LISTED: Mutex<Vec<Listed>> = Mutex::new(Vec::new());

/// Every camera AVFoundation lists, in its order, kept as the last listing.
fn list() -> Vec<Listed> {
    let found = listed().unwrap_or_default();
    found.clone_into(&mut LISTED.lock().unwrap_or_else(PoisonError::into_inner));
    found
}

/// The device monitor's cameras, or `None` where GStreamer cannot be asked.
fn listed() -> Option<Vec<Listed>> {
    gst::init().ok()?;
    // Ours before any monitor loads a system's NDI plugin in its place: see `video::ndi`.
    let _ = crate::video::ndi::register();
    let monitor = gst::DeviceMonitor::new();
    monitor.add_filter(Some("Video/Source"), None);
    monitor.start().ok()?;
    let found = monitor
        .devices()
        .iter()
        .filter_map(|d| {
            let id = d.properties()?.get::<String>("avf.unique_id").ok()?;
            // Made and dropped in the null state: read, never started.
            let index = d.create_element(None).ok()?.property::<i32>("device-index");
            Some(Listed {
                id,
                name: d.display_name().to_string(),
                index,
                caps: d.caps(),
            })
        })
        .collect();
    monitor.stop();
    Some(found)
}

/// Every camera AVFoundation lists, as `(unique ID, name)`, in its order.
pub fn capture_devices() -> Vec<(String, String)> {
    list().into_iter().map(|c| (c.id, c.name)).collect()
}

/// The first camera AVFoundation lists.
pub fn first_capture_device() -> Option<String> {
    list().into_iter().next().map(|c| c.id)
}

/// The camera with this unique ID as the last listing has it, or as a fresh listing does when
/// the last one lacks it.
fn find(id: &str) -> Option<Listed> {
    let find = |listed: &[Listed]| listed.iter().find(|c| c.id == id).cloned();
    let known = find(&LISTED.lock().unwrap_or_else(PoisonError::into_inner));
    known.or_else(|| find(&list()))
}

/// The source element for the camera with this unique ID, held to frames in system memory:
/// looked up in the last listing, and listed again when it is not there. An error names an
/// ID no camera here has.
pub fn device_element(id: &str) -> Result<String, String> {
    let camera = find(id).ok_or_else(|| format!("no camera {id} on this Mac"))?;
    Ok(source_element(camera.index))
}

/// The formats and sizes the camera with this unique ID offers, GL textures among them, from
/// the listing, which opens no camera.
pub fn device_caps(id: &str) -> Option<gst::Caps> {
    find(id)?.caps
}

/// `avfvideosrc` on the camera at `index`, with the caps that keep it out of GL memory.
fn source_element(index: i32) -> String {
    format!("avfvideosrc device-index={index} ! {IN_MEMORY}")
}

/// Raw video in system memory, which is every structure `avfvideosrc` lists but its GL
/// textures.
const IN_MEMORY: &str = "video/x-raw";

/// Nothing: decodebin's own order already puts `vtdec_hw` first.
pub fn prefer_hardware_jpeg(_decodebin: &gst::Element) {}

/// The hardware codecs a clip's cache may be written with, in order of preference; every
/// Apple Silicon Mac has both, and `vtdec_hw` decodes both. `max-keyframe-interval=1` makes
/// every frame an IDR, and `quality` with `bitrate` left at 0 is constant quality.
pub const CODECS: &[Codec] = &[
    Codec {
        name: "h264",
        encoder: "vtenc_h264_hw",
        decoder: "vtdec_hw",
        parser: "h264parse",
        intra: "max-keyframe-interval=1 quality=0.7",
        delivery: "quality=0.7 allow-frame-reordering=false",
    },
    Codec {
        name: "h265",
        encoder: "vtenc_h265_hw",
        decoder: "vtdec_hw",
        parser: "h265parse",
        intra: "max-keyframe-interval=1 quality=0.7",
        delivery: "quality=0.7 allow-frame-reordering=false",
    },
];

/// What a status line names when [`CODECS`] has nothing this machine can run.
pub const CODEC_HINT: &str = "VideoToolbox";

/// How long the end of a stream waits on a codec that has stopped handing frames back. Far
/// longer than a frame takes even with the machine busy, so a stall is VideoToolbox keeping a
/// frame on purpose or having dropped it, not one being slow.
const SETTLE_STALL: Duration = Duration::from_secs(1);

/// Hold the end of the stream at each VideoToolbox codec in `pipeline`, now or added later by
/// a `decodebin`, until VideoToolbox has handed back every frame it was given, so the drain
/// the end starts has nothing still in flight to lose.
///
/// **`vtenc` and `vtdec` can drop the frames still in flight when the stream ends**
/// (GStreamer 1.28's `applemedia`, unchanged on its main branch). Both drain the same way:
/// set a draining flag, wait for VideoToolbox (`VTCompressionSessionCompleteFrames`,
/// `VTDecompressionSessionWaitForAsynchronousFrames`), then pause the output task from
/// outside. A frame VideoToolbox hands back during that wait lands in the output queue after
/// the task's last pass found it empty, and the pause strands it there until the next flush
/// throws it away. Under load a clip's last frame went missing that way about one encode in
/// forty, a delivery's last one or two one in ten, and an import's source decode lost its last
/// frame about one time in fifty.
///
/// An encoder is waited on until it holds no frames at all. VideoToolbox hands a frame back in
/// a few milliseconds without being asked when it has no reason to keep it, which is why the
/// delivery rows turn frame reordering off; `vtenc` itself keeps the very first frame until a
/// second one arrives, so a stream of one frame is not waited on. A decoder is waited on until
/// every frame it holds has been decoded: `vtdec` keeps up to its picture buffer's depth of
/// them back to put them in order, and only the drain lets those go. Either way a codec that
/// stops moving for [`SETTLE_STALL`] is let go to drain as it would have without this.
///
/// This does not cover `vtdec`'s other loss, an output task that has not run at all since a
/// seek by the time the stream ends: `clip.rs` asks again for a frame that came back short.
pub fn settle_before_eos(pipeline: &gst::Pipeline) {
    for element in pipeline.iterate_recurse().into_iter().flatten() {
        settle(&element);
    }
    pipeline.connect_deep_element_added(|_, _, element| settle(element));
}

/// Hold the end of the stream at `element` if it is one of VideoToolbox's codecs.
fn settle(element: &gst::Element) {
    let Some(factory) = element.factory() else {
        return;
    };
    let name = factory.name();
    let Some(pad) = element.static_pad("sink") else {
        return;
    };
    if name.starts_with("vtenc") {
        let Some(encoder) = element.downcast_ref::<gst_video::VideoEncoder>() else {
            return;
        };
        // Weak: the probe lives on the codec's own pad, and a strong reference would keep it
        // alive forever.
        let encoder = encoder.downgrade();
        hold_eos(&pad, 2, move || Some(encoder.upgrade()?.frames().len()));
    } else if name.starts_with("vtdec") {
        let Some(decoder) = element.downcast_ref::<gst_video::VideoDecoder>() else {
            return;
        };
        let decoder = decoder.downgrade();
        hold_eos(&pad, 1, move || {
            // A frame still inside VideoToolbox has no picture yet; one decoded and held back
            // for ordering has. The callback that fills it in writes the pointer without the
            // decoder's lock, so this reads it only for whether it is there.
            let decoder = decoder.upgrade()?;
            let frames = decoder.frames();
            Some(
                frames
                    .iter()
                    .filter(|f| f.output_buffer().is_none())
                    .count(),
            )
        });
    }
}

/// A probe on `pad` that, once at least `least` buffers have gone through it, holds the end of
/// the stream while `in_flight` says frames are still inside the codec and still coming back.
fn hold_eos(
    pad: &gst::Pad,
    least: u64,
    in_flight: impl Fn() -> Option<usize> + Send + Sync + 'static,
) {
    let taken = AtomicU64::new(0);
    pad.add_probe(
        gst::PadProbeType::BUFFER | gst::PadProbeType::EVENT_DOWNSTREAM,
        move |_, info| {
            match &info.data {
                Some(gst::PadProbeData::Buffer(_)) => {
                    taken.fetch_add(1, Ordering::Relaxed);
                }
                Some(gst::PadProbeData::Event(e))
                    if e.type_() == gst::EventType::Eos
                        && taken.load(Ordering::Relaxed) >= least =>
                {
                    let mut pending = in_flight().unwrap_or(0);
                    let mut moved = Instant::now();
                    while pending > 0 && moved.elapsed() < SETTLE_STALL {
                        std::thread::sleep(Duration::from_millis(1));
                        let now = in_flight().unwrap_or(0);
                        if now < pending {
                            moved = Instant::now();
                        }
                        pending = now;
                    }
                }
                _ => {}
            }
            gst::PadProbeReturn::Ok
        },
    );
}

/// `None`: no DMA-BUF is exported here.
pub fn dmabuf_format() -> Option<String> {
    None
}

/// Can a decoded clip reach the GPU without a copy here: wherever the renderer's device
/// imports an `IOSurface`.
pub fn clip_dmabuf() -> bool {
    dmabuf_imports()
}

/// Can the renderer's device import an `IOSurface`. False before there is a renderer.
pub fn dmabuf_imports() -> bool {
    crate::render::dmabuf::imports()
}

/// The bytes chain, whose `videoconvert` passes `vtdec_hw`'s NV12 through untouched, so the
/// sample is the decoder's own `CVPixelBuffer`.
pub fn dmabuf_chain() -> Option<String> {
    Some(format!(
        "videoconvert ! video/x-raw,format={{{}}}",
        crate::video::UPLOADABLE
    ))
}

/// `None`: a screen is not a pipeline here.
pub fn dmabuf_caps() -> Option<gst::Caps> {
    None
}

/// A sample in a VideoToolbox decoder's own memory as a `Frame`: the `IOSurface` behind it,
/// and the same buffer mapped as bytes, held for as long as the frame is. `None` for a buffer
/// with no `IOSurface` behind it, which the caller maps as bytes.
pub fn dmabuf_frame(caps: &gst::CapsRef, buffer: &gst::BufferRef) -> Option<Result<Frame, String>> {
    let surface = super::pixels::surface_of(buffer)?;
    Some(
        crate::video::clip::mapped(caps, buffer).and_then(|frame| match frame.pixels {
            Pixels::Mapped(mapped) => Ok(Frame {
                pixels: Pixels::IoSurface(IoSurface {
                    surface,
                    mapped,
                    redrawn: None,
                }),
                ..frame
            }),
            _ => Err("a mapped buffer that is not mapped".to_string()),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only prints. Whether a camera exists is a fact about the machine, not the code.
    #[test]
    fn capture_devices_can_be_listed() {
        for (id, name) in capture_devices() {
            println!("{id}: {name}");
        }
        println!("first capture device: {:?}", first_capture_device());
    }

    /// The caps after the camera take its frames in memory at any size and refuse its GL
    /// textures, which `avfvideosrc` lists first. A FaceTime HD camera's first two structures,
    /// as its device lists them.
    #[test]
    fn a_camera_is_held_to_frames_in_memory() {
        gst::init().expect("gstreamer");
        let element = source_element(0);
        let filter: gst::Caps = element
            .rsplit(" ! ")
            .next()
            .and_then(|caps| caps.parse().ok())
            .expect("caps after the source");
        let gl: gst::Caps = "video/x-raw(memory:GLMemory), width=1552, height=1552, \
                             format={ UYVY, YUY2 }, framerate=[ 15/1, 30/1 ], \
                             texture-target=rectangle"
            .parse()
            .expect("caps");
        let memory: gst::Caps = "video/x-raw, width=1552, height=1552, \
                                 format={ UYVY, YUY2, NV12, ARGB, BGRA }, framerate=[ 15/1, 30/1 ]"
            .parse()
            .expect("caps");
        assert!(!filter.can_intersect(&gl), "{element}");
        assert!(filter.can_intersect(&memory), "{element}");
    }

    /// An ID no camera has is an error naming it, after a listing that opens nothing.
    #[test]
    fn an_id_no_camera_has_is_refused_by_name() {
        let e = device_element("no-such-camera").unwrap_err();
        assert!(e.contains("no-such-camera"), "{e}");
    }
}
