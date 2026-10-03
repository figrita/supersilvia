// SPDX-License-Identifier: AGPL-3.0-or-later

//! Video sources, through GStreamer.
//!
//! A `Camera` is one pipeline ending in an `appsink`. GStreamer captures and decodes on its
//! own threads; the sink's callback wraps each frame in an `Arc` and leaves it in a one-frame
//! slot. `tick` takes the newest one, and `render/` uploads it once — the same `Arc` seen
//! twice is skipped.
//!
//! Nothing converts a frame on the CPU. A camera's frames arrive in system memory in the
//! device's own layout — YUY2, NV12, I420, or the planes an MJPEG decoder writes — and are
//! held as they are, mapped rather than copied; `render/` uploads them as they lie and
//! converts to RGB in a shader. A screen cast and a decoded file are exported as DMA-BUF
//! descriptors where the machine can, a decoded file as Direct3D 12 textures on Windows, and
//! `render/dmabuf.rs` imports them. Neither held
//! buffer reaches `render/` as a GStreamer type: a mapped one is behind
//! [`Planes`](crate::nodes::Planes), and the keep-alive behind a descriptor is an opaque
//! `Arc`.
//!
//! **One slot, not a triple buffer.** Every frame is the source's own buffer, from a pool the
//! source cannot refill while it is held, and a triple buffer keeps two stale frames alive in
//! its spare slots. A screen cast with three buffers would then stop until the next frame
//! arrived, which it never could. The slot holds only the newest unread frame; the sink
//! writes it under a lock held for a pointer's swap, and `latest` only ever tries the lock.
//! See [docs/media.md](../../docs/media.md#cameras).
//!
//! **A device is opened once, whoever reads it.** A second reader of a camera — another
//! Camera node, the Main Input — joins the pipeline already open on it, with a slot of its
//! own the sink writes each frame into beside the first reader's. Opening it again is wrong
//! on both machines: V4L2 refuses a second capture as busy, and AVFoundation starts a second
//! session that sets the device's format for both, so the first pipeline goes on describing
//! frames by caps that no longer fit them. A reader asking for a size other than the one the
//! device was opened at is refused rather than handed another size.
//!
//! **A reader asking for no size opens a camera at 1920x1080, then 1280x720,** where the
//! device's listing offers one — the device monitor's caps, read without opening it — and
//! otherwise at the first size the device lists, which on a Mac's FaceTime HD camera is a
//! 1552x1552 square. The size chosen is the pipeline's, so a later reader asking for another
//! is refused by it.
//!
//! **The pipelines are the same on every machine; the elements at their ends are not.** A
//! camera's source, a screen's, the hardware codecs and the zero-copy export are named by
//! [`crate::platform::video`] and [`crate::platform::screen`] — V4L2, PipeWire, VA-API and
//! DMA-BUF on Linux — and everything between them is here. A screen on macOS has no pipeline
//! at all: ScreenCaptureKit writes its frames into the same one-frame slot itself, and the
//! `Camera` over it only reads.

pub mod clip;
pub mod encode;
pub mod gif;
pub mod ndi;
pub mod png;
pub mod silence;
pub mod syphon;
pub mod text;

use crate::nodes::{Choices, Frame, Pixels};
use crate::platform;
use crate::platform::screen::Head;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

/// What the Camera node's menu offers with no camera listed.
const NO_CAMERAS: Choices = &[("auto", "Auto"), ("test", "Test pattern")];

/// The Camera node's menu as the last listing left it.
static CAMERA_MENU: Mutex<Choices> = Mutex::new(NO_CAMERAS);

/// Every capture device the machine has, as `(ID, name)`, in its order. It asks the machine,
/// which costs a device monitor started and stopped, so only the synth's Main Input calls it,
/// once and again on *Look for devices again*. Where cameras go by name, the Camera node's
/// menu is set from the answer.
pub fn capture_devices() -> Vec<(String, String)> {
    let found = platform::video::capture_devices();
    if platform::video::NAMED_CAMERAS {
        let mut menu = CAMERA_MENU.lock().unwrap_or_else(PoisonError::into_inner);
        // Between *Auto* and the test pattern.
        let listed = &menu[1..menu.len() - 1];
        let same = listed.len() == found.len()
            && listed
                .iter()
                .zip(&found)
                .all(|(&(id, name), (i, n))| id == i.as_str() && name == n.as_str());
        if !same {
            // Leaked once a listing that differs, which is a hand asking again.
            let leak = |s: &str| &*Box::leak(s.to_owned().into_boxed_str());
            let listed = found.iter().map(|(id, name)| (leak(id), leak(name)));
            let choices: Vec<_> = NO_CAMERAS[..1]
                .iter()
                .copied()
                .chain(listed)
                .chain(NO_CAMERAS[1..].iter().copied())
                .collect();
            *menu = Box::leak(choices.into_boxed_slice());
        }
    }
    found
}

/// The Camera node's menu: *Auto*, each camera the last listing found by name, and the test
/// pattern. Empty where cameras are saved by a path the node's own choices name, so the menu
/// is those.
pub fn camera_menu() -> Choices {
    if !platform::video::NAMED_CAMERAS {
        return &[];
    }
    *CAMERA_MENU.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What a sentence calls the camera with this ID: its name where the last listing has one,
/// and the ID itself where the ID is what a person reads.
fn camera_name(id: &str) -> String {
    camera_menu()
        .iter()
        .find(|&&(i, _)| i == id)
        .map_or_else(|| id.to_string(), |&(_, name)| name.to_string())
}

/// How a menu shows a camera listed as `(id, name)`.
pub fn camera_label(id: &str, name: &str) -> String {
    if platform::video::NAMED_CAMERAS {
        name.to_string()
    } else {
        format!("{name} — {id}")
    }
}

/// How frames leave a pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// Bytes in system memory in the source's own layout, held rather than copied, and
    /// converted to RGB by the renderer on the GPU. `videoconvert` stays in the chain only
    /// for a format outside [`UPLOADABLE`] and passes everything else through untouched.
    /// Works everywhere.
    Bytes,
    /// Tightly packed RGBA8, converted by `videoconvert` on the CPU. For a reader that wants
    /// pixels of its own — a poster, the letters of a string — and not for anything uploaded
    /// every frame.
    Rgba,
    /// A frame in GPU memory, never copied; the renderer samples it directly. On Linux a
    /// DMA-BUF: from a decoder converted to RGBA by `vapostproc`, which needs VA-API, and from a
    /// screen cast the compositor's own buffer, in whichever RGB format and tiling it and the
    /// renderer's device agree on, the cast falling back to bytes when they agree on none. On
    /// a Mac a decoder's `IOSurface`, and on Windows a Direct3D 12 texture on the renderer's
    /// own device.
    DmaBuf,
}

/// How a delivery reads where a person sees it, a node's line in the Status box: the GPU's own
/// buffer is "Zero copy" on every system, a DMA-BUF on Linux, an `IOSurface` on a Mac and a
/// Direct3D 12 texture on Windows.
impl std::fmt::Display for Delivery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Delivery::Bytes => "bytes",
            Delivery::Rgba => "RGBA",
            Delivery::DmaBuf => "Zero copy",
        })
    }
}

/// The formats the renderer uploads as they are: four byte orders of RGB, and YUV planar,
/// semi-planar and packed. In the order a source that can make any of them is asked,
/// RGBA first, so the test pattern arrives as bytes a test can read.
pub const UPLOADABLE: &str = "RGBA,BGRA,RGBx,BGRx,NV12,I420,YV12,Y42B,Y444,YUY2,UYVY";

/// The sizes a camera is opened at when its reader asks for none, in order of preference.
const PREFERRED_SIZES: [(u32, u32); 2] = [(1920, 1080), (1280, 720)];

/// Caps for a camera's frames at one size, raw or MJPEG: a webcam offers 720p as MJPEG far
/// more often than as raw, and decodebin handles both.
fn size_caps(w: u32, h: u32) -> String {
    format!("video/x-raw,width={w},height={h};image/jpeg,width={w},height={h}")
}

/// The first of [`PREFERRED_SIZES`] that `offered`, a device's caps as its listing gives them,
/// carries raw or as JPEG in system memory, or `None` where it carries neither.
fn preferred_size(offered: &gst::Caps) -> Option<(u32, u32)> {
    PREFERRED_SIZES.into_iter().find(|&(w, h)| {
        size_caps(w, h)
            .parse::<gst::Caps>()
            .is_ok_and(|caps| offered.can_intersect(&caps))
    })
}

/// The appsink every chain ends in: keep one frame, never pace to a clock.
const APPSINK: &str = "appsink name=sink sync=false max-buffers=1 drop=true";

/// The sink end of a pipeline: an appsink that keeps one frame and never paces to a clock,
/// preceded by the conversion `delivery` asks for.
fn sink_chain(delivery: Delivery) -> Option<String> {
    Some(match delivery {
        Delivery::Bytes => {
            format!("videoconvert ! video/x-raw,format={{{UPLOADABLE}}} ! {APPSINK}")
        }
        Delivery::Rgba => format!("videoconvert ! video/x-raw,format=RGBA ! {APPSINK}"),
        Delivery::DmaBuf => format!("{} ! {APPSINK}", platform::video::dmabuf_chain()?),
    })
}

/// What a screen cast's appsink accepts when it asks for DMA-BUFs: the RGB formats and
/// tilings the renderer's device imports, then the same bytes a camera delivers.
///
/// `pipewiresrc` offers the compositor both, so the choice is the compositor's and a
/// compositor that shares only memory is taken in memory, with nothing to retry. `None`
/// where nothing is importable here — there is no renderer yet, or none of its formats is
/// one this reads — which is the bytes-only chain.
fn screen_caps() -> Option<gst::Caps> {
    let mut caps = platform::video::dmabuf_caps()?;
    let bytes: gst::Caps = format!("video/x-raw,format={{{UPLOADABLE}}}")
        .parse()
        .ok()?;
    caps.merge(bytes);
    Some(caps)
}

/// `vapostproc` refuses to hand out DMA-BUFs to a sink that has not said it understands
/// `VideoMeta`, which is where the stride and offset travel. This is the allocation answer
/// appsink gives on our behalf.
pub(crate) fn propose_video_meta(
    _sink: &gst_app::AppSink,
    query: &mut gst::query::Allocation,
) -> bool {
    query.add_allocation_meta::<gst_video::VideoMeta>(None);
    true
}

/// Where a camera pipeline starts. The rest of the pipeline is the same for every source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A capture device by the name the machine gives it: a V4L2 node such as `/dev/video0`
    /// on Linux.
    Device(String),
    /// A screen the desktop handed over. The [`Cast`](crate::platform::screen::Cast) this
    /// came from must outlive the pipeline — it is what holds the capture open, and the
    /// stream is only valid while it does.
    Screen(platform::screen::Stream),
    /// A Syphon server's frames, written by a client. The
    /// [`Inlet`](crate::platform::syphon::Inlet) this came from must outlive the camera.
    Syphon(platform::syphon::Stream),
    /// An NDI source on the network, by the name NDI gives it, `MACHINE (Stream)`: `ndisrc`
    /// and its demuxer, the picture alone (`ndi::video_head`).
    Ndi(String),
    /// The first device that can capture.
    Auto,
    /// GStreamer's test pattern. Real frames without a camera, which is what the tests use.
    Test,
    /// A source written out as a pipeline fragment and decoded as a device's output is: a
    /// test pattern in a format a test chooses, or one encoded to MJPEG the way a webcam
    /// sends it.
    Described(String),
}

impl Source {
    /// This source with `Auto` replaced by the device it means, so a device chosen by name
    /// and the same device reached through `Auto` are one source.
    fn resolved(&self) -> Result<Self, String> {
        match self {
            // Not the source element's own default: on Linux that is /dev/video0, which on a
            // desktop with OBS or a loopback module is an output-only device that cannot
            // capture.
            Self::Auto => platform::video::first_capture_device()
                .map(Self::Device)
                .ok_or_else(|| "no video capture device found".to_string()),
            other => Ok(other.clone()),
        }
    }

    /// What a pipeline on this resolved source is shared under, or `None` for one every
    /// reader opens for itself: a test pattern, and a screen, whose capture is each reader's
    /// own. A described source is shared as the device it stands in for, and an NDI source by
    /// its name, apart from every device.
    fn shared_key(&self) -> Option<String> {
        match self {
            Self::Device(id) | Self::Described(id) => Some(id.clone()),
            Self::Ndi(name) => Some(format!("ndi:{name}")),
            Self::Auto | Self::Test | Self::Screen(_) | Self::Syphon(_) => None,
        }
    }

    /// Why a reader asking for `size` cannot join this source's pipeline, opened at `open`.
    fn busy(&self, open: Option<(u32, u32)>) -> String {
        let name = match self {
            Self::Device(id) => camera_name(id),
            Self::Described(head) => head.clone(),
            Self::Ndi(name) => name.clone(),
            Self::Auto | Self::Test | Self::Screen(_) | Self::Syphon(_) => "the source".to_string(),
        };
        match open {
            Some((w, h)) => format!(
                "{name} is open elsewhere at {w}x{h}: set Size to {w}x{h} or Auto to share it"
            ),
            None => {
                format!("{name} is open elsewhere at its own size: set Size to Auto to share it")
            }
        }
    }

    /// The size a pipeline on this resolved source opens at for a reader asking for none: the
    /// first preferred size the device's listing offers, or `None`, which takes the first
    /// size the device lists.
    fn unasked_size(&self) -> Option<(u32, u32)> {
        match self {
            Self::Device(id) => preferred_size(&platform::video::device_caps(id)?),
            Self::Auto
            | Self::Test
            | Self::Screen(_)
            | Self::Syphon(_)
            | Self::Ndi(_)
            | Self::Described(_) => None,
        }
    }

    /// Where this source's frames come from: an element at the head of a pipeline, or, for a
    /// screen the machine delivers without GStreamer, the capture's own slots.
    fn head(&self) -> Result<Head, String> {
        Ok(Head::Element(match self {
            Self::Device(path) => platform::video::device_element(path)?,
            Self::Auto => return self.resolved()?.head(),
            Self::Screen(stream) => return Ok(stream.head()),
            Self::Syphon(stream) => return Ok(stream.head()),
            Self::Ndi(name) => ndi::video_head(name),
            Self::Test => "videotestsrc is-live=true".to_string(),
            Self::Described(head) => head.clone(),
        }))
    }

    /// Does what comes out of this source need decoding?
    ///
    /// A camera hands over MJPEG as often as raw and a test pattern is raw, so both go through
    /// `decodebin`, which passes raw through untouched. A screen cast is raw by construction —
    /// the compositor is handing over frames, not a file — so it has nothing to typefind and
    /// nothing to decode, and `decodebin` in front of a live source is only a stage to get
    /// wrong. The same holds for a Syphon client's frames and for an NDI source, which `ndisrc`
    /// decompresses itself.
    fn decoded(&self) -> bool {
        !matches!(self, Self::Screen(_) | Self::Syphon(_) | Self::Ndi(_))
    }
}

/// One reader's handoff: the newest frame the sink has written and the reader has not taken.
type Handoff = Mutex<Option<Arc<Frame>>>;

/// A running pipeline and everything its readers share: the slots its sink writes, the error
/// it reported, and what it was opened with. Stopped when the last reader lets go of it.
struct Capture {
    pipeline: gst::Pipeline,
    /// Every reader's slot. The sink writes each frame into each one whose reader is alive.
    readers: Arc<Mutex<Vec<Weak<Handoff>>>>,
    /// An error the sink or the bus reported after start-up.
    error: Arc<Mutex<Option<String>>>,
    /// Raised by the renderer when it could not import one of this pipeline's DMA-BUFs.
    refused: Arc<AtomicBool>,
    /// What [`OPEN`] finds it under, for a pipeline a second reader may join.
    key: Option<String>,
    size: Option<(u32, u32)>,
    description: String,
    delivery: Delivery,
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

/// Every device pipeline running, which a second reader of the same device joins.
static OPEN: Mutex<Vec<Weak<Capture>>> = Mutex::new(Vec::new());

/// A reader of a running capture: a pipeline, which the last reader dropping stops, or the
/// slots of a capture the machine delivers itself, which the capture's own handle stops.
pub struct Camera {
    /// `None` for a capture that writes its slots itself.
    capture: Option<Arc<Capture>>,
    /// The newest frame the sink has written and `latest` has not taken.
    slot: Arc<Handoff>,
    /// The last frame handed out, so `latest` can answer "nothing new" by pointer.
    current: Option<Arc<Frame>>,
    /// An error the sink or the bus reported after start-up.
    error: Arc<Mutex<Option<String>>>,
    /// Raised by the renderer when it could not import one of this pipeline's DMA-BUFs.
    refused: Arc<AtomicBool>,
    /// What was opened, so a refused DMA-BUF delivery can be reopened as bytes.
    source: Source,
    size: Option<(u32, u32)>,
    /// Whether this pipeline has handed over a frame yet.
    delivered: bool,
    pub description: String,
    pub delivery: Delivery,
}

impl Camera {
    /// Build and start a pipeline, or read the one already running on the same device.
    /// `size` constrains the source's caps; `None` takes whatever size a running pipeline was
    /// opened at, or opens the device at 1920x1080 or 1280x720 where its listing offers one
    /// and at the first size it lists otherwise. A size other than a running pipeline's is an
    /// error naming it.
    ///
    /// A screen cast asks for DMA-BUFs where this machine can import any; everything else
    /// delivers bytes, because a USB webcam's frames arrive in system memory whatever is
    /// asked.
    pub fn open(source: &Source, size: Option<(u32, u32)>) -> Result<Self, String> {
        let delivery = if matches!(source, Source::Screen(_))
            && platform::video::dmabuf_imports()
            && screen_caps().is_some()
        {
            Delivery::DmaBuf
        } else {
            Delivery::Bytes
        };
        Self::open_with(source, size, delivery)
    }

    /// Build and start a pipeline with the given delivery.
    ///
    /// `DmaBuf` from a device or a test pattern moves the conversion onto the GPU through the
    /// machine's zero-copy chain — `vapostproc` on Linux, `d3d12upload` on Windows — and hands
    /// the pipeline what that chain needs; it exists so the import can be tested against the
    /// test pattern.
    /// From a screen it asks the compositor for its own buffers and takes bytes if it has
    /// none to give. A screen whose capture writes its frames itself builds no pipeline, and
    /// `size` and `delivery` are the capture's. Only a pipeline delivering bytes is shared.
    pub fn open_with(
        source: &Source,
        size: Option<(u32, u32)>,
        delivery: Delivery,
    ) -> Result<Self, String> {
        gst::init().map_err(|e| format!("gstreamer: {e}"))?;
        let source = &source.resolved()?;
        let key = match delivery {
            Delivery::Bytes => source.shared_key(),
            Delivery::Rgba | Delivery::DmaBuf => None,
        };
        let Some(key) = key else {
            return Self::start(source, size, delivery, None);
        };
        // Held while the pipeline starts, so two readers opening one device at once open it
        // once.
        let mut open = OPEN.lock().unwrap_or_else(PoisonError::into_inner);
        open.retain(|capture| capture.strong_count() > 0);
        if let Some(capture) = open
            .iter()
            .filter_map(Weak::upgrade)
            .find(|capture| capture.key.as_deref() == Some(key.as_str()))
        {
            if size.is_some() && size != capture.size {
                return Err(source.busy(capture.size));
            }
            return Ok(Self::join(source, capture));
        }
        let camera = Self::start(source, size, delivery, Some(key))?;
        open.extend(camera.capture.as_ref().map(Arc::downgrade));
        Ok(camera)
    }

    /// Build and start a pipeline on `source`, found again under `key` where it is shared,
    /// and read it.
    fn start(
        source: &Source,
        size: Option<(u32, u32)>,
        delivery: Delivery,
        key: Option<String>,
    ) -> Result<Self, String> {
        let element = match source.head()? {
            Head::Element(element) => element,
            Head::Slots {
                frame,
                error,
                description,
            } => return Ok(Self::adopt(source, frame, error, description)),
        };
        let screen = matches!(source, Source::Screen(_));
        let size = size.or_else(|| source.unasked_size());
        let caps = match size {
            Some((w, h)) if !screen => format!(" ! capsfilter caps=\"{}\"", size_caps(w, h)),
            _ => String::new(),
        };
        // `sync=false`: show a frame when it exists, not when its timestamp says. `drop`
        // with one buffer: the newest frame wins, never a queue of stale ones.
        let tail = match (screen, delivery) {
            // The appsink's caps say DMA-BUF or bytes; they are set below, because the list
            // of formats and tilings is built rather than written.
            (true, Delivery::DmaBuf) => APPSINK.to_string(),
            // A camera's frames are in system memory, so the DMA-BUF chain starts with a
            // conversion into a format the video engine will upload; it passes through when
            // the format already fits.
            (false, Delivery::DmaBuf) => format!(
                "videoconvert ! {}",
                sink_chain(delivery).ok_or("no DMA-BUF export on this machine")?
            ),
            _ => sink_chain(delivery).ok_or("no sink chain")?,
        };
        let description = format!(
            "{element}{caps}{} ! {tail}",
            if source.decoded() {
                " ! decodebin name=decode"
            } else {
                ""
            },
        );
        let pipeline = gst::parse::launch(&description)
            .map_err(|e| format!("{description}: {e}"))?
            .downcast::<gst::Pipeline>()
            .map_err(|_| "not a pipeline".to_string())?;
        if delivery == Delivery::DmaBuf {
            platform::video::dmabuf_context(&pipeline);
        }
        let sink = pipeline
            .by_name("sink")
            .ok_or("no appsink")?
            .downcast::<gst_app::AppSink>()
            .map_err(|_| "sink is not an appsink".to_string())?;
        if screen && delivery == Delivery::DmaBuf {
            sink.set_caps(Some(
                &screen_caps().ok_or("nothing this machine can import")?,
            ));
        }
        if let Some(decode) = pipeline.by_name("decode") {
            platform::video::prefer_hardware_jpeg(&decode);
        }

        let readers: Arc<Mutex<Vec<Weak<Handoff>>>> = Arc::default();
        let error = Arc::new(Mutex::new(None));
        let refused = Arc::new(AtomicBool::new(false));
        let (sink_readers, sink_error, sink_refused) = (
            Arc::clone(&readers),
            Arc::clone(&error),
            Arc::clone(&refused),
        );
        sink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .propose_allocation(propose_video_meta)
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    match frame_from(&sample, delivery) {
                        Ok(mut frame) => {
                            match &mut frame.pixels {
                                Pixels::DmaBuf(buf) => {
                                    buf.refused = Some(Arc::clone(&sink_refused));
                                }
                                Pixels::D3d12(texture) => {
                                    texture.refused = Some(Arc::clone(&sink_refused));
                                }
                                _ => {}
                            }
                            let frame = Arc::new(frame);
                            let readers =
                                sink_readers.lock().unwrap_or_else(PoisonError::into_inner);
                            for reader in readers.iter().filter_map(Weak::upgrade) {
                                // The frame this displaces is dropped after the slot's lock is
                                // let go, so its buffer goes back to the source outside it.
                                let displaced = match reader.lock() {
                                    Ok(mut slot) => slot.replace(Arc::clone(&frame)),
                                    Err(_) => None,
                                };
                                drop(displaced);
                            }
                        }
                        Err(e) => {
                            if let Ok(mut slot) = sink_error.lock() {
                                *slot = Some(e);
                            }
                        }
                    }
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        let capture = Arc::new(Capture {
            pipeline,
            readers,
            error,
            refused,
            key,
            size,
            description,
            delivery,
        });
        capture
            .pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| format!("{}: {e}", capture.description))?;
        Ok(Self::join(source, capture))
    }

    /// A reader of `capture`, with a slot of its own that the sink writes every frame into.
    fn join(source: &Source, capture: Arc<Capture>) -> Self {
        let slot: Arc<Handoff> = Arc::new(Mutex::new(None));
        {
            let mut readers = capture
                .readers
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            readers.retain(|reader| reader.strong_count() > 0);
            readers.push(Arc::downgrade(&slot));
        }
        Self {
            slot,
            current: None,
            error: Arc::clone(&capture.error),
            refused: Arc::clone(&capture.refused),
            source: source.clone(),
            size: capture.size,
            delivered: false,
            description: capture.description.clone(),
            delivery: capture.delivery,
            capture: Some(capture),
        }
    }

    /// A capture that writes `frame` and `error` itself, as the sink's callback writes a
    /// pipeline's: the same one-frame slot, and the reason it ended.
    fn adopt(
        source: &Source,
        frame: platform::screen::Slot<Arc<Frame>>,
        error: platform::screen::Slot<String>,
        description: String,
    ) -> Self {
        Self {
            capture: None,
            slot: frame,
            current: None,
            error,
            refused: Arc::new(AtomicBool::new(false)),
            source: source.clone(),
            size: None,
            delivered: false,
            description,
            // The capture's frames are in memory the GPU shares, which the renderer imports
            // where it can and uploads where it cannot; nothing is reopened either way.
            delivery: Delivery::DmaBuf,
        }
    }

    /// The newest frame, or `None` if the source has produced nothing yet. Never waits: a
    /// sink in the middle of writing the slot is "nothing new" until the next call.
    pub fn latest(&mut self) -> Option<Arc<Frame>> {
        if self.delivery == Delivery::DmaBuf && self.capture.is_some() {
            if self.refused.swap(false, Ordering::Relaxed) {
                self.reopen_as_bytes("the renderer refused its DMA-BUFs");
            } else if self.unnegotiated() {
                self.reopen_as_bytes("the source and the sink agreed on no format");
            }
        }
        if let Ok(mut slot) = self.slot.try_lock()
            && let Some(frame) = slot.take()
        {
            self.current = Some(frame);
            self.delivered = true;
        }
        self.current.clone()
    }

    /// Whether a screen cast asking for DMA-BUFs failed before its first frame — which is a
    /// negotiation that found nothing, since the caps also offer bytes, and is worth one more
    /// try with the bytes chain and its `videoconvert`. The error is taken off the bus here.
    fn unnegotiated(&self) -> bool {
        matches!(self.source, Source::Screen(_))
            && !self.delivered
            && self
                .pipeline()
                .and_then(gst::Pipeline::bus)
                .and_then(|bus| bus.pop_filtered(&[gst::MessageType::Error]))
                .is_some()
    }

    /// What this pipeline exports cannot be used — the renderer could not import it, or no
    /// format was agreed — so the same source is opened again delivering bytes. The last
    /// frame stays up until the first of those arrives.
    fn reopen_as_bytes(&mut self, why: &str) {
        log::warn!("{}: {why}; delivering bytes", self.description);
        // Stopped first, so the source is never read by two pipelines at once.
        if let Some(pipeline) = self.pipeline() {
            let _ = pipeline.set_state(gst::State::Null);
        }
        match Self::open_with(&self.source, self.size, Delivery::Bytes) {
            Ok(mut camera) => {
                camera.current = self.current.take();
                *self = camera;
            }
            Err(e) => {
                if let Ok(mut slot) = self.error.lock() {
                    *slot = Some(e);
                }
            }
        }
    }

    /// An error since start-up, from the sink, the bus, or a capture that writes its own.
    /// Polled, since there is no GLib main loop to watch the bus for us.
    pub fn error(&self) -> Option<String> {
        let bus = self.pipeline().and_then(gst::Pipeline::bus);
        // End-of-stream counts. A camera unplugged and a screen cast the desktop stopped —
        // its own "stop sharing" button — both arrive as EOS rather than as an error, and
        // without this the source simply freezes on its last frame and says nothing.
        if let Some(bus) = &bus
            && let Some(msg) = bus.pop_filtered(&[gst::MessageType::Eos])
            && matches!(msg.view(), gst::MessageView::Eos(_))
            && let Ok(mut slot) = self.error.lock()
        {
            *slot = Some("the source stopped".to_string());
        }
        if let Some(bus) = &bus
            && let Some(msg) = bus.pop_filtered(&[gst::MessageType::Error])
            && let gst::MessageView::Error(e) = msg.view()
            && let Ok(mut slot) = self.error.lock()
        {
            *slot = Some(format!(
                "{}: {}",
                e.src().map_or_else(
                    || "pipeline".into(),
                    gstreamer::prelude::GstObjectExt::path_string
                ),
                e.error()
            ));
        }
        self.error.lock().ok().and_then(|s| s.clone())
    }

    /// The factory names of every element in the pipeline, decodebin's own included, and
    /// none for a capture with no pipeline. For tests that ask which decoder was chosen.
    pub fn elements(&self) -> Vec<String> {
        self.pipeline()
            .into_iter()
            .flat_map(|p| p.iterate_recurse().into_iter().flatten())
            .filter_map(|e| e.factory().map(|f| f.name().to_string()))
            .collect()
    }

    /// The pipeline this reads, which other readers of the same device may read too.
    fn pipeline(&self) -> Option<&gst::Pipeline> {
        self.capture.as_ref().map(|capture| &capture.pipeline)
    }
}

use clip::frame_from;

#[cfg(test)]
mod tests {
    use super::*;

    /// A delivery in the GPU's own memory reads "Zero copy" where a person sees it, on both
    /// systems, rather than the name of Linux's buffer.
    #[test]
    fn a_gpu_delivery_reads_zero_copy() {
        assert_eq!(Delivery::DmaBuf.to_string(), "Zero copy");
        assert_eq!(Delivery::Bytes.to_string(), "bytes");
    }

    /// The test pattern is a real pipeline through the real appsink; only the source
    /// differs from a camera. A frame has to arrive, be RGBA, and be tightly packed.
    #[test]
    fn the_test_pattern_delivers_packed_rgba_frames() {
        let mut camera = Camera::open(&Source::Test, Some((320, 240))).expect("pipeline");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let frame = loop {
            if let Some(f) = camera.latest() {
                break f;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "no frame within five seconds: {:?}",
                camera.error()
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!((frame.width, frame.height), (320, 240));
        let bytes = frame.bytes().expect("bytes were asked for");
        assert_eq!(bytes.len(), 320 * 240 * 4);
        // The default pattern is color bars; the top-left bar is white, opaque.
        assert_eq!(&bytes[0..4], &[255, 255, 255, 255]);
        assert_eq!(camera.error(), None);
    }

    /// The same pipeline exporting a DMA-BUF: a descriptor, a format, a modifier, and the
    /// buffer held behind it. Only where the machine can export one.
    #[test]
    fn the_test_pattern_can_be_delivered_as_a_dmabuf() {
        if platform::video::dmabuf_format().is_none() {
            eprintln!("no vapostproc RGBA export here; skipping");
            return;
        }
        let mut camera =
            Camera::open_with(&Source::Test, Some((320, 240)), Delivery::DmaBuf).expect("pipeline");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let frame = loop {
            if let Some(f) = camera.latest() {
                break f;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "no frame within five seconds: {:?}",
                camera.error()
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        let crate::nodes::Pixels::DmaBuf(buf) = &frame.pixels else {
            panic!("expected a DMA-BUF, got bytes");
        };
        assert!(buf.fd >= 0);
        assert!(buf.stride >= 320 * 4);
        assert_ne!(buf.fourcc, 0);
        assert_eq!(camera.error(), None);
    }

    fn first_frame(camera: &mut Camera) -> Arc<Frame> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(f) = camera.latest() {
                return f;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "no frame within five seconds: {:?}",
                camera.error()
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// Two readers of one device read one pipeline: the second joins the first, both are
    /// handed frames, the first leaving does not stop the second, and a size other than the
    /// running pipeline's is refused by name. A described source stands in for the device.
    #[test]
    fn a_second_reader_of_a_device_joins_its_pipeline() {
        let source = Source::Described("videotestsrc is-live=true pattern=ball".to_string());
        let mut first = Camera::open(&source, Some((160, 120))).expect("pipeline");
        let mut second = Camera::open(&source, None).expect("joined");
        assert!(
            Arc::ptr_eq(
                first.capture.as_ref().expect("a pipeline"),
                second.capture.as_ref().expect("a pipeline")
            ),
            "one pipeline"
        );
        first_frame(&mut first);
        let seen = first_frame(&mut second);
        assert_eq!(
            (seen.width, seen.height),
            (160, 120),
            "the size it was opened at"
        );

        let Err(e) = Camera::open(&source, Some((320, 240))) else {
            panic!("another size joined a running pipeline");
        };
        assert!(e.contains("160x120"), "{e}");
        drop(Camera::open(&source, Some((160, 120))).expect("the same size joins"));

        drop(first);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(f) = second.latest()
                && !Arc::ptr_eq(&f, &seen)
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the second reader stopped with the first: {:?}",
                second.error()
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(second.error(), None);

        drop(second);
        let mut again = Camera::open(&source, Some((320, 240))).expect("opened again");
        let frame = first_frame(&mut again);
        assert_eq!(
            (frame.width, frame.height),
            (320, 240),
            "a pipeline of its own"
        );
    }

    /// The FaceTime HD camera of an M2 MacBook Air, as the device monitor lists it: every size
    /// as a GL texture first, then the same sizes in memory, a 1552x1552 square first.
    const FACETIME: &str = "\
        video/x-raw(memory:GLMemory), width=1552, height=1552, format={ UYVY, YUY2 }, framerate=[ 15/1, 30/1 ], texture-target=rectangle; \
        video/x-raw(memory:GLMemory), width=1920, height=1080, format={ UYVY, YUY2 }, framerate=[ 15/1, 30/1 ], texture-target=rectangle; \
        video/x-raw, width=1552, height=1552, format={ UYVY, YUY2, NV12, ARGB, BGRA }, framerate=[ 15/1, 30/1 ]; \
        video/x-raw, width=1328, height=1760, format={ UYVY, YUY2, NV12, ARGB, BGRA }, framerate=[ 15/1, 30/1 ]; \
        video/x-raw, width=640, height=480, format={ UYVY, YUY2, NV12, ARGB, BGRA }, framerate=[ 15/1, 30/1 ]; \
        video/x-raw, width=1280, height=720, format={ UYVY, YUY2, NV12, ARGB, BGRA }, framerate=[ 15/1, 30/1 ]; \
        video/x-raw, width=1920, height=1080, format={ UYVY, YUY2, NV12, ARGB, BGRA }, framerate=[ 15/1, 30/1 ]";

    fn preferred(caps: &str) -> Option<(u32, u32)> {
        gst::init().expect("gstreamer");
        preferred_size(&caps.parse().expect("caps"))
    }

    /// 1920x1080 wherever it is offered in a form the pipeline takes, whatever is listed first;
    /// then 1280x720; then nothing, which leaves the camera at the first size it lists.
    #[test]
    fn a_camera_asked_for_no_size_prefers_1080p_then_720p() {
        assert_eq!(preferred(FACETIME), Some((1920, 1080)));
        // A height listed as a choice, as an iPhone's Continuity Camera lists it.
        assert_eq!(
            preferred("video/x-raw, width=1920, height={ 1440, 1080 }, format=NV12"),
            Some((1920, 1080))
        );
        // A V4L2 webcam that sends its large sizes only as MJPEG.
        assert_eq!(
            preferred(
                "video/x-raw, format=YUY2, width=640, height=480; \
                 image/jpeg, width=1920, height=1080, framerate=30/1"
            ),
            Some((1920, 1080))
        );
        assert_eq!(
            preferred(
                "video/x-raw, width=1552, height=1552, format=UYVY; \
                 video/x-raw, width=1280, height=720, format=UYVY"
            ),
            Some((1280, 720))
        );
        assert_eq!(
            preferred(
                "video/x-raw, width=1552, height=1552, format=UYVY; \
                 video/x-raw, width=640, height=480, format=UYVY"
            ),
            None
        );
        // Offered only as a GL texture, which the camera's pipeline refuses.
        assert_eq!(
            preferred(
                "video/x-raw(memory:GLMemory), width=1920, height=1080, format=UYVY; \
                 video/x-raw, width=1552, height=1552, format=UYVY"
            ),
            None
        );
    }

    /// The size chosen from the FaceTime listing negotiates behind a source held to memory, as
    /// the Mac's camera is, with format and framerate left to the source.
    #[test]
    fn the_preferred_size_negotiates_behind_a_source_held_to_memory() {
        let size = preferred(FACETIME);
        let head = "videotestsrc is-live=true ! video/x-raw".to_string();
        let mut camera = Camera::open(&Source::Described(head), size).expect("pipeline");
        let frame = first_frame(&mut camera);
        assert_eq!((frame.width, frame.height), (1920, 1080));
        assert_eq!(camera.error(), None);
    }

    /// A source that makes YUV is delivered as YUV, in its own buffer at its own stride:
    /// nothing converted it and nothing copied it.
    #[test]
    fn a_yuv_source_is_held_as_it_lies() {
        let head =
            "videotestsrc is-live=true ! video/x-raw,format=I420,width=190,height=96".to_string();
        let mut camera = Camera::open(&Source::Described(head), None).expect("pipeline");
        let frame = first_frame(&mut camera);
        let Pixels::Mapped(m) = &frame.pixels else {
            panic!("expected the mapped buffer, got {:?}", frame.pixels);
        };
        assert_eq!(m.layout, crate::nodes::Layout::I420);
        assert_eq!(m.strides, [192, 96, 96], "the pipeline's own padding");
        for i in 0..3 {
            assert!(m.plane(i, 190, 96).is_some(), "plane {i} is all there");
        }
        assert!(frame.bytes().is_none(), "not RGBA, so no bytes to read");
    }

    /// An MJPEG stream, as a webcam sends one, is decoded by the video engine where
    /// `vajpegdec` or `vtdec_hw` exists and by `jpegdec` where neither does — and either way
    /// arrives as the decoder's own planes. `vtdec_hw` is macOS's, and decodebin ranks it
    /// above `jpegdec` on its own, with no preference asked of it.
    #[test]
    fn mjpeg_decodes_on_the_video_engine_where_it_can() {
        let head = "videotestsrc is-live=true ! video/x-raw,format=Y42B,width=320,height=240 \
                    ! jpegenc"
            .to_string();
        let mut camera = Camera::open(&Source::Described(head), None).expect("pipeline");
        let frame = first_frame(&mut camera);
        let Pixels::Mapped(m) = &frame.pixels else {
            panic!("expected the mapped buffer, got {:?}", frame.pixels);
        };
        assert!(m.layout.is_yuv(), "{:?}", m.layout);
        let elements = camera.elements();
        if gst::ElementFactory::find("vajpegdec").is_some() {
            assert!(
                elements.iter().any(|e| e == "vajpegdec"),
                "decoded on the video engine: {elements:?}"
            );
            assert!(!elements.iter().any(|e| e == "jpegdec"), "{elements:?}");
        } else if gst::ElementFactory::find("vtdec_hw").is_some() {
            assert!(
                elements.iter().any(|e| e == "vtdec_hw"),
                "decoded on the video engine: {elements:?}"
            );
            assert!(!elements.iter().any(|e| e == "jpegdec"), "{elements:?}");
        } else {
            assert!(elements.iter().any(|e| e == "jpegdec"), "{elements:?}");
        }
        assert_eq!(camera.error(), None);
    }
}
