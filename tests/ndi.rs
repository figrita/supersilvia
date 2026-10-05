// SPDX-License-Identifier: AGPL-3.0-or-later

//! NDI®'s loopback: a four-quadrant picture sent through `appsrc ! ndisink` and received by
//! `ndisrc ! ndisrcdemux` in the same process, over the machine's own network stack, through
//! the plugin `video::ndi` registers.
//!
//! **What it settles**, once the NDI runtime is installed: that a picture sent top row first
//! arrives top row first; that one sent with alpha arrives as BGRA with its alpha, and one sent
//! opaque as UYVY, `ndisrc`'s default; and how long a frame takes to come back, printed. Then
//! the same through the app's own halves: an Output's picture sent by the publisher's thread
//! (`render::publish`), drawn and read back off the GPU, arriving top row first with its alpha,
//! straight as NDI defines it; and that stream listed by `video::ndi::labels` and received
//! through `video::ndi::Receiver`, a camera, with its alpha or read opaque.
//!
//! **Without the runtime it skips**, saying so: the runtime is proprietary and the user's to
//! install (`proposals/ndi.md`), so a machine without it — any CI box, and a Mac nobody put it
//! on — passes by doing nothing past registering the plugin.

#[path = "common/gpu.rs"]
mod gpu;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use gstreamer_video::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const W: u32 = 256;
const H: u32 = 144;

/// The four quadrants, as `[B, G, R, A]`: red, green, blue, and a bottom right that is
/// transparent black where alpha is sent.
const QUADRANTS: [[u8; 4]; 4] = [
    [0, 0, 255, 255],
    [0, 255, 0, 255],
    [255, 0, 0, 255],
    [0, 0, 0, 0],
];

/// The picture top row first, BGRA, its quadrants turned a quarter where `turned`.
fn pattern(turned: bool) -> Vec<u8> {
    let mut bytes = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        for x in 0..W {
            let mut q = usize::from(x >= W / 2) + 2 * usize::from(y >= H / 2);
            if turned {
                q = [1, 3, 0, 2][q];
            }
            bytes.extend_from_slice(&QUADRANTS[q]);
        }
    }
    bytes
}

/// The publisher's four quadrants as they should arrive, `[B, G, R, A]` and straight, as NDI
/// defines BGRA: red, green, blue, and a bottom right of full red at half alpha, which the
/// Output's picture holds premultiplied.
const PUBLISHED: [[u8; 4]; 4] = [
    [0, 0, 255, 255],
    [0, 255, 0, 255],
    [255, 0, 0, 255],
    [0, 0, 255, 128],
];

/// Where each quadrant's middle is.
const MIDDLES: [(u32, u32); 4] = [
    (W / 4, H / 4),
    (3 * W / 4, H / 4),
    (W / 4, 3 * H / 4),
    (3 * W / 4, 3 * H / 4),
];

fn until<T>(what: &str, within: Duration, mut f: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + within;
    loop {
        if let Some(t) = f() {
            return t;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A sender pushing the pattern at 30 frames a second from a thread of its own, `alpha` saying
/// whether as BGRA or BGRx, until dropped.
struct Sender {
    pipeline: gst::Pipeline,
    turned: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Sender {
    fn new(stream: &str, alpha: bool) -> Self {
        let format = if alpha { "BGRA" } else { "BGRx" };
        let pipeline = gst::parse::launch(&format!(
            "appsrc name=src is-live=true format=time do-timestamp=true \
             caps=video/x-raw,format={format},width={W},height={H},framerate=30/1 \
             ! ndisink ndi-name=\"{stream}\" sync=false"
        ))
        .expect("the sending pipeline")
        .downcast::<gst::Pipeline>()
        .expect("a pipeline");
        let src = pipeline
            .by_name("src")
            .expect("appsrc")
            .downcast::<gst_app::AppSrc>()
            .expect("an appsrc");
        pipeline
            .set_state(gst::State::Playing)
            .expect("the sender starts");
        let turned = Arc::new(AtomicBool::new(false));
        let running = Arc::new(AtomicBool::new(true));
        let (t, r) = (Arc::clone(&turned), Arc::clone(&running));
        let thread = std::thread::spawn(move || {
            let (plain, quarter) = (pattern(false), pattern(true));
            while r.load(Ordering::Relaxed) {
                let bytes = if t.load(Ordering::Relaxed) {
                    &quarter
                } else {
                    &plain
                };
                let _ = src.push_buffer(gst::Buffer::from_slice(bytes.clone()));
                std::thread::sleep(Duration::from_millis(33));
            }
        });
        Self {
            pipeline,
            turned,
            running,
            thread: Some(thread),
        }
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

/// The network name NDI gives `stream` on this machine, `MACHINE (stream)`, found by the
/// plugin's device provider, started by name since no device monitor reaches it. **It is left
/// running**: the provider is one per process, and `gst-plugin-ndi` 0.15's `stop` leaves its
/// find instance behind, so a provider started again never polls — which would starve
/// `video::ndi::labels` below, as the app, which never stops it, is not.
fn network_name(stream: &str) -> String {
    let provider =
        gst::DeviceProviderFactory::by_name("ndideviceprovider").expect("the NDI device provider");
    provider.start().expect("the NDI device provider starts");
    let suffix = format!("({stream})");
    until(
        "the device provider listing the sender",
        Duration::from_secs(15),
        || {
            provider
                .devices()
                .iter()
                .map(|d| d.display_name().to_string())
                .find(|n| n.ends_with(&suffix))
        },
    )
}

/// A frame as the receiver hands it: its format and its bytes by row.
struct Received {
    format: gst_video::VideoFormat,
    stride: usize,
    bytes: Vec<u8>,
}

impl Received {
    /// The four bytes at pixel `(x, y)` of a BGRA frame.
    fn bgra(&self, x: u32, y: u32) -> [u8; 4] {
        let at = y as usize * self.stride + x as usize * 4;
        self.bytes[at..at + 4].try_into().expect("four bytes")
    }

    /// The luma at pixel `(x, y)` of a UYVY frame.
    fn luma(&self, x: u32, y: u32) -> u8 {
        let pair = y as usize * self.stride + (x as usize / 2) * 4;
        self.bytes[pair + 1 + 2 * (x as usize % 2)]
    }
}

/// A receiver of `name`, keeping the newest frame, as a camera's appsink does.
struct Receiver {
    pipeline: gst::Pipeline,
    newest: Arc<Mutex<Option<(Instant, Received)>>>,
}

impl Receiver {
    fn new(name: &str) -> Self {
        let pipeline = gst::parse::launch(&format!(
            "ndisrc ndi-name=\"{name}\" ! ndisrcdemux name=demux demux.video \
             ! appsink name=sink sync=false max-buffers=1 drop=true"
        ))
        .expect("the receiving pipeline")
        .downcast::<gst::Pipeline>()
        .expect("a pipeline");
        let sink = pipeline
            .by_name("sink")
            .expect("appsink")
            .downcast::<gst_app::AppSink>()
            .expect("an appsink");
        let newest: Arc<Mutex<Option<(Instant, Received)>>> = Arc::default();
        let slot = Arc::clone(&newest);
        sink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    let caps = sample.caps().ok_or(gst::FlowError::Error)?;
                    let info =
                        gst_video::VideoInfo::from_caps(caps).map_err(|_| gst::FlowError::Error)?;
                    let buffer = sample.buffer_owned().ok_or(gst::FlowError::Error)?;
                    let frame = gst_video::VideoFrame::from_buffer_readable(buffer, &info)
                        .map_err(|_| gst::FlowError::Error)?;
                    let received = Received {
                        format: info.format(),
                        stride: frame.plane_stride()[0] as usize,
                        bytes: frame
                            .plane_data(0)
                            .map_err(|_| gst::FlowError::Error)?
                            .to_vec(),
                    };
                    *slot.lock().expect("the slot") = Some((Instant::now(), received));
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
        pipeline
            .set_state(gst::State::Playing)
            .expect("the receiver starts");
        Self { pipeline, newest }
    }

    /// The newest frame that arrived after `since`, taken.
    fn after(&self, since: Instant) -> Option<Received> {
        let mut slot = self.newest.lock().expect("the slot");
        if slot.as_ref().is_some_and(|(at, _)| *at > since) {
            return slot.take().map(|(_, r)| r);
        }
        None
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

fn near(a: u8, b: u8) -> bool {
    a.abs_diff(b) <= 40
}

fn holds(frame: &Received, turned: bool) -> bool {
    holds_quadrants(frame, turned, &QUADRANTS)
}

fn holds_quadrants(frame: &Received, turned: bool, quadrants: &[[u8; 4]; 4]) -> bool {
    MIDDLES.iter().enumerate().all(|(q, &(x, y))| {
        let q = if turned { [1, 3, 0, 2][q] } else { q };
        let got = frame.bgra(x, y);
        got.iter().zip(quadrants[q]).all(|(&g, w)| near(g, w))
    })
}

#[test]
fn a_picture_sent_over_ndi_comes_back_the_right_way_up() {
    supersilvia::video::ndi::register().expect("the plugin registers");
    if !supersilvia::video::ndi::runtime() {
        eprintln!(
            "ndi: {}; skipping the loopback",
            supersilvia::video::ndi::missing().unwrap_or(supersilvia::video::ndi::MISSING)
        );
        return;
    }
    let started = Instant::now();
    let stream = format!("supersilvia loopback {}", std::process::id());

    // With alpha: BGRA out, BGRA back, top row first, the transparent quadrant transparent.
    let sender = Sender::new(&stream, true);
    let name = network_name(&stream);
    println!("ndi: the loopback sender is {name:?}");
    let receiver = Receiver::new(&name);
    let first = until("a frame back", Duration::from_secs(15), || {
        receiver.after(started)
    });
    assert_eq!(
        first.format,
        gst_video::VideoFormat::Bgra,
        "alpha comes back"
    );
    assert!(
        holds(&first, false),
        "top row first, alpha kept: {:?}",
        MIDDLES.map(|(x, y)| first.bgra(x, y))
    );

    // The delay: from the pattern turning to the first frame back that shows it.
    let turned_at = Instant::now();
    sender.turned.store(true, Ordering::Relaxed);
    until("the turned picture back", Duration::from_secs(5), || {
        receiver.after(turned_at).filter(|f| holds(f, true))
    });
    println!(
        "ndi: a frame comes back {:.0} ms after it is sent",
        turned_at.elapsed().as_secs_f64() * 1000.0
    );
    drop(receiver);
    drop(sender);

    // Opaque: BGRx out, and back as UYVY, `ndisrc`'s default for a stream with no alpha,
    // still top row first — green the brightest, then red, then blue.
    let stream = format!("{stream} opaque");
    let _sender = Sender::new(&stream, false);
    let receiver = Receiver::new(&network_name(&stream));
    let frame = until("an opaque frame back", Duration::from_secs(15), || {
        receiver.after(started)
    });
    assert_eq!(frame.format, gst_video::VideoFormat::Uyvy, "no alpha, UYVY");
    let [red, green, blue, _] = MIDDLES.map(|(x, y)| frame.luma(x, y));
    assert!(
        green > red && red > blue,
        "top row first: {red} {green} {blue}"
    );
    println!("ndi: opaque comes back as UYVY, top row first");
    drop(receiver);

    publisher(&format!(
        "supersilvia loopback {} Output 7",
        std::process::id()
    ));
}

/// [`PUBLISHED`] as an Output's picture would be one: a texture on the GPU, RGBA and
/// premultiplied, uploaded top row first as a CPU node's is.
fn picture(gpu: &supersilvia::render::Gpu) -> supersilvia::render::Picture {
    let premultiply = |c: u8, a: u8| ((u16::from(c) * u16::from(a) + 127) / 255) as u8;
    let mut rgba = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        for x in 0..W {
            let [b, g, r, a] = PUBLISHED[usize::from(x >= W / 2) + 2 * usize::from(y >= H / 2)];
            rgba.extend_from_slice(&[premultiply(r, a), premultiply(g, a), premultiply(b, a), a]);
        }
    }
    let texture = wgpu::util::DeviceExt::create_texture_with_data(
        gpu.device(),
        gpu.queue(),
        &wgpu::TextureDescriptor {
            label: Some("pattern"),
            size: wgpu::Extent3d {
                width: W,
                height: H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &rgba,
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    supersilvia::render::Picture {
        texture: supersilvia::render::Texture::new(texture, view),
        width: W,
        height: H,
        flip: true,
        sampler: supersilvia::compile::wgsl::Sampler::MirrorNearest,
        drawn_tick: None,
    }
}

/// The publisher's thread, as the app runs it: Output 7's picture set in a `Live` thirty times
/// a second, as the synth sets it, sent as `name` with its alpha, and received top row first
/// and straight.
fn publisher(name: &str) {
    use supersilvia::graph::NodeId;
    use supersilvia::render::picture::Shown;
    use supersilvia::render::publish::{Look, Publisher, Via, Wanted};
    use supersilvia::render::{Live, Published};

    let gpu = gpu::gpu();
    let live: Arc<Live> = Arc::default();
    let running = Arc::new(AtomicBool::new(true));
    let setter = {
        let (live, running, base) = (Arc::clone(&live), Arc::clone(&running), picture(&gpu));
        std::thread::spawn(move || {
            let mut tick = 0;
            while running.load(Ordering::Relaxed) {
                tick += 1;
                let mut picture = base.clone();
                picture.drawn_tick = Some(tick);
                live.set(Arc::new(Published {
                    tick,
                    outputs: std::collections::HashMap::from([(NodeId(7), picture)]),
                    ..Published::default()
                }));
                std::thread::sleep(Duration::from_millis(33));
            }
        })
    };
    let mut publisher = Publisher::new(Some(&gpu), Arc::clone(&live));
    publisher.want(vec![Wanted {
        picture: Shown::Node {
            node: NodeId(7),
            port: None,
        },
        via: Via::Ndi { rate: 30 },
        name: name.to_string(),
        look: Look {
            flip: false,
            transparent: true,
        },
    }]);
    let started = Instant::now();
    let receiver = Receiver::new(&network_name(name));
    let frame = until(
        "the publisher's frame back",
        Duration::from_secs(15),
        || {
            receiver
                .after(started)
                .filter(|f| holds_quadrants(f, false, &PUBLISHED))
        },
    );
    assert_eq!(
        frame.format,
        gst_video::VideoFormat::Bgra,
        "alpha comes back"
    );
    println!("ndi: the publisher sends an Output top row first, with its alpha, straight");
    drop(receiver);

    // And received as the Main Input and the NDI node receive: listed, opened as a camera,
    // its alpha kept where Transparent says and its BGRA read as BGRx where it does not.
    let listed = until("the app's listing", Duration::from_secs(15), || {
        supersilvia::video::ndi::labels()
            .into_iter()
            .find(|n| n.ends_with(&format!("({name})")))
    });
    for (transparent, layout) in [
        (true, supersilvia::nodes::Layout::Bgra),
        (false, supersilvia::nodes::Layout::Bgrx),
    ] {
        let mut receiver = supersilvia::video::ndi::Receiver::new(&listed, transparent);
        let frame = until(
            "a frame through the camera",
            Duration::from_secs(15),
            || receiver.latest(),
        );
        let supersilvia::nodes::Pixels::Mapped(mapped) = &frame.pixels else {
            panic!("a mapped frame: {:?}", frame.pixels);
        };
        assert_eq!(mapped.layout, layout, "transparent {transparent}");
        assert_eq!((frame.width, frame.height), (W, H));
        assert!(receiver.receiving() && receiver.error().is_none());
    }
    println!("ndi: a source is received through the camera, opaque or with its alpha");
    publisher.stop();
    running.store(false, Ordering::Relaxed);
    let _ = setter.join();
}
