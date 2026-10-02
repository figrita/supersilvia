// SPDX-License-Identifier: AGPL-3.0-or-later

//! Syphon's loopback: a picture published through `render::syphon` and received in the same
//! process through Syphon's own client, byte for byte, found in between by the framework's own
//! directory; then the publisher's thread, and the receiving half the Main Input and the Syphon
//! node run, through to the renderer's copy.
//!
//! **The process's `main`, not libtest's.** The directory hears servers announce themselves on
//! the main thread's run loop, which libtest's threads never run, so `Cargo.toml` turns the
//! harness off and this pumps that loop itself. Each check prints a line; the first that fails
//! panics, which fails the run.
//!
//! **What it settles.** A picture drawn with the default [`Look`] reaches a client with its
//! **bottom row first** in the surface's memory, and opaque over black; flipped and
//! transparent, with its top row first and its own alpha. Received, it is copied into a texture
//! of ours top row first, opaque or with its alpha. Linux has no Syphon: the directory is empty
//! and a server is refused.

#[cfg(target_os = "macos")]
#[path = "common/gpu.rs"]
mod gpu;

fn main() {
    #[cfg(target_os = "macos")]
    mac::loopback();
    #[cfg(not(target_os = "macos"))]
    refused();
}

/// Linux answers as a machine without Syphon.
#[cfg(not(target_os = "macos"))]
fn refused() {
    use supersilvia::platform::syphon::{Server, servers};
    assert!(servers().is_empty(), "no server is ever listed");
    assert!(Server::new("loopback").is_err(), "a server is refused");
    println!("syphon: refused here, as a machine without it");
}

#[cfg(target_os = "macos")]
mod mac {
    use super::gpu;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    use supersilvia::compile::wgsl::Sampler;
    use supersilvia::graph::NodeId;
    use supersilvia::platform::syphon::{Client, Surface, pump, servers};
    use supersilvia::render::picture::Shown;
    use supersilvia::render::publish::{Look, Publisher, Via, Wanted};
    use supersilvia::render::syphon::Outlet;
    use supersilvia::render::{
        FrameJob, Live, Picture, Published, Renderer, SourceJob, Texture, Viewer,
    };

    /// The picture's size: wide enough that each quadrant is several pixels.
    const W: u32 = 8;
    const H: u32 = 4;

    /// Each quadrant as RGBA, premultiplied: red over green on the left of the top, blue and
    /// half-covered white along the bottom.
    const TOP_LEFT: [u8; 4] = [255, 0, 0, 255];
    const TOP_RIGHT: [u8; 4] = [0, 255, 0, 255];
    const BOTTOM_LEFT: [u8; 4] = [0, 0, 255, 255];
    const BOTTOM_RIGHT: [u8; 4] = [128, 128, 128, 128];

    /// How long anything here waits for the framework before the check fails.
    const PATIENCE: Duration = Duration::from_secs(10);

    /// The quadrant pattern at `W` by `H`, uploaded top row first, as a CPU node's frame is,
    /// drawn on tick `tick`.
    fn pattern(gpu: &supersilvia::render::Gpu, (w, h): (u32, u32), tick: u64) -> Picture {
        let mut bytes = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                bytes.extend_from_slice(&match (y < h / 2, x < w / 2) {
                    (true, true) => TOP_LEFT,
                    (true, false) => TOP_RIGHT,
                    (false, true) => BOTTOM_LEFT,
                    (false, false) => BOTTOM_RIGHT,
                });
            }
        }
        let texture = wgpu::util::DeviceExt::create_texture_with_data(
            gpu.device(),
            gpu.queue(),
            &wgpu::TextureDescriptor {
                label: Some("pattern"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
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
            &bytes,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Picture {
            texture: Texture::new(texture, view),
            width: w,
            height: h,
            flip: true,
            sampler: Sampler::MirrorNearest,
            drawn_tick: Some(tick),
        }
    }

    /// Pump the run loop until `done` answers, or fail saying `what`.
    fn until<T>(what: &str, mut done: impl FnMut() -> Option<T>) -> T {
        let start = Instant::now();
        loop {
            if let Some(v) = done() {
                return v;
            }
            assert!(
                start.elapsed() < PATIENCE,
                "{what} took more than {PATIENCE:?}"
            );
            pump(Duration::from_millis(20));
        }
    }

    /// BGRA, as the surface holds it.
    fn bgra([r, g, b, a]: [u8; 4]) -> [u8; 4] {
        [b, g, r, a]
    }

    /// The pixel at column `x` of row `row` of a surface `width` pixels wide.
    fn at(bytes: &[u8], width: u32, row: u32, x: u32) -> [u8; 4] {
        let i = ((row * width + x) * 4) as usize;
        bytes[i..i + 4].try_into().unwrap()
    }

    /// Whether `surface` holds the pattern at its own size bottom row first, opaque.
    fn holds_the_pattern_bottom_first(surface: &Surface) -> bool {
        let (w, h) = surface.size();
        let bytes = surface.read().expect("the surface reads");
        let seen = |row: u32, x: u32| at(&bytes, w, row, x);
        seen(0, 0) == bgra(BOTTOM_LEFT)
            && seen(0, w - 1) == bgra(opaque(BOTTOM_RIGHT))
            && seen(h - 1, 0) == bgra(TOP_LEFT)
            && seen(h - 1, w - 1) == bgra(TOP_RIGHT)
    }

    /// Opaque over black: what the half-covered quadrant is once its alpha is one.
    fn opaque(rgba: [u8; 4]) -> [u8; 4] {
        [rgba[0], rgba[1], rgba[2], 255]
    }

    pub fn loopback() {
        let gpu = gpu::gpu();
        let viewer = Viewer::new(&gpu, wgpu::TextureFormat::Bgra8Unorm).expect("the blit");
        let picture = pattern(&gpu, (W, H), 1);
        let mut outlet = Outlet::new("loopback").expect("a server starts");
        let me = outlet.server().described();
        println!("syphon: serving {:?} as {:?}", me.name, me.label());

        let found = until("the directory hearing the server", || {
            servers().into_iter().find(|s| s.id == me.id)
        });
        assert_eq!(found.name, "loopback");
        assert!(!found.app.is_empty(), "the app is named: {found:?}");
        println!("syphon: the directory lists {:?}", found.label());

        let (tx, frames) = mpsc::channel::<Surface>();
        let client = Client::new(&found, move |surface| {
            let _ = tx.send(surface);
        })
        .expect("a client connects");

        for (look, bottom_first, alpha) in [
            (Look::default(), true, false),
            (
                Look {
                    flip: true,
                    transparent: true,
                },
                false,
                true,
            ),
        ] {
            while frames.try_recv().is_ok() {}
            let ticket = outlet.draw(&gpu, &viewer, &picture, look).expect("drawn");
            gpu.wait(&ticket, PATIENCE)
                .expect("the GPU finishes the blit");
            // Published until the client has heard one: its subscription reaches the server
            // over a port of its own, a moment after it is made.
            let surface = until("a frame reaching the client", || {
                outlet.publish();
                frames.try_recv().ok()
            });
            assert!(client.is_valid());
            assert_eq!(surface.size(), (W, H));
            assert_eq!(
                surface.pixel_format(),
                u32::from_be_bytes(*b"BGRA"),
                "the framework names its surface's format"
            );
            let bytes = surface.read().expect("the surface reads");
            let (first, last) = if bottom_first {
                ([BOTTOM_LEFT, BOTTOM_RIGHT], [TOP_LEFT, TOP_RIGHT])
            } else {
                ([TOP_LEFT, TOP_RIGHT], [BOTTOM_LEFT, BOTTOM_RIGHT])
            };
            let seen = |row: u32, x: u32| at(&bytes, W, row, x);
            let want = |rgba: [u8; 4]| bgra(if alpha { rgba } else { opaque(rgba) });
            for (row, pair) in [(0, first), (H - 1, last)] {
                assert_eq!(seen(row, 0), want(pair[0]), "{look:?}: row {row}, left");
                assert_eq!(
                    seen(row, W - 1),
                    want(pair[1]),
                    "{look:?}: row {row}, right"
                );
            }
            println!(
                "syphon: {look:?} reaches the client {} row first, {}",
                if bottom_first { "bottom" } else { "top" },
                if alpha {
                    "with the picture's own alpha"
                } else {
                    "opaque over black"
                }
            );
        }
        drop(client);
        drop(outlet);
        until("the directory hearing the server retire", || {
            servers().iter().all(|s| s.id != me.id).then_some(())
        });
        println!("syphon: the server retires when it drops");
        publisher(&gpu);
        receiver(&gpu);
    }

    /// Set `picture` as Output 7's frame in `live`, as the synth publishes one.
    fn publish(live: &Live, picture: Picture) {
        let tick = picture.drawn_tick.unwrap_or(0);
        live.set(Arc::new(Published {
            tick,
            outputs: HashMap::from([(NodeId(7), picture)]),
            ..Published::default()
        }));
    }

    /// The publisher's thread, as the app runs it: an Output's frame read from `Live`, drawn
    /// for a client, a new size reaching the client already drawn, and the server retired once
    /// nothing wants it.
    fn publisher(gpu: &supersilvia::render::Gpu) {
        let live: Arc<Live> = Arc::default();
        let mut tick = 1;
        publish(&live, pattern(gpu, (W, H), tick));
        let mut publisher = Publisher::new(Some(gpu), Arc::clone(&live));
        publisher.want(vec![Wanted {
            picture: Shown::Node {
                node: NodeId(7),
                port: None,
            },
            via: Via::Syphon,
            name: "Output 7".to_string(),
            look: Look::default(),
        }]);
        let found = until("the directory hearing the publisher's server", || {
            servers().into_iter().find(|s| s.name == "Output 7")
        });
        println!("syphon: the publisher serves {:?}", found.label());
        let (tx, frames) = mpsc::channel::<Surface>();
        let client = Client::new(&found, move |surface| {
            let _ = tx.send(surface);
        })
        .expect("a client connects");

        // Each tick a new frame, as the synth sets them; nothing is drawn until a client reads.
        let mut next = |size: (u32, u32)| {
            until("a frame of the new size reaching the client", || {
                tick += 1;
                publish(&live, pattern(gpu, size, tick));
                frames.try_iter().find(|s| s.size() == size)
            })
        };
        let first = next((W, H));
        assert!(
            holds_the_pattern_bottom_first(&first),
            "the Output's frame, bottom row first"
        );
        let bigger = next((W * 2, H * 3));
        assert!(
            holds_the_pattern_bottom_first(&bigger),
            "a new size reaches the client already drawn: zero flash"
        );
        println!("syphon: the publisher draws each new frame, and a new size arrives drawn");

        publisher.want(Vec::new());
        until("the directory hearing an unwanted server retire", || {
            servers().iter().all(|s| s.id != found.id).then_some(())
        });
        drop(client);
        publisher.stop();
        println!("syphon: a server no longer wanted retires");
    }

    /// The receiving half as the Main Input and the Syphon node run it: a server found by its
    /// label, frames landing in a camera's slot, and the renderer's copy of each — top row first
    /// and opaque by default, as its own app drew it, and with its alpha where asked.
    fn receiver(gpu: &supersilvia::render::Gpu) {
        use supersilvia::graph::PortRef;
        use supersilvia::platform::syphon::Look as Reading;
        use supersilvia::video::syphon::{Receiver, labels, menu};

        let viewer = Viewer::new(gpu, wgpu::TextureFormat::Bgra8Unorm).expect("the blit");
        let picture = pattern(gpu, (W, H), 1);
        let mut outlet = Outlet::new("received").expect("a server starts");
        let label = outlet.server().described().label();
        until("the listing naming the server", || {
            labels().contains(&label).then_some(())
        });
        assert!(
            menu().iter().any(|&(value, _)| value == label),
            "the node's menu offers it"
        );
        let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
        for (reading, bottom_right) in [
            (Reading::default(), opaque(BOTTOM_RIGHT)),
            (
                Reading {
                    flip: false,
                    transparent: true,
                },
                BOTTOM_RIGHT,
            ),
        ] {
            let mut receiver = Receiver::new(&label, reading);
            let frame = until("a frame reaching the receiver", || {
                // Sent with the alpha the reading keeps, so there is some to keep.
                let sent = Look {
                    flip: false,
                    transparent: reading.transparent,
                };
                let ticket = outlet.draw(gpu, &viewer, &picture, sent).ok()?;
                gpu.wait(&ticket, PATIENCE).ok()?;
                outlet.publish();
                receiver.latest()
            });
            assert!(receiver.connected() && receiver.error().is_none());
            let node = NodeId(9);
            renderer.draw(&FrameJob {
                sources: vec![SourceJob::new(PortRef::new(node, "frame"), frame)],
                ..FrameJob::default()
            });
            gpu::drain(gpu);
            let bytes = gpu::bytes_of(gpu, &renderer.texture_of(node).expect("copied"));
            let rgba = |row: u32, x: u32| at(&bytes, W, row, x);
            assert_eq!(rgba(0, 0), TOP_LEFT, "{reading:?}: the top row first");
            assert_eq!(rgba(0, W - 1), TOP_RIGHT, "{reading:?}: top right");
            assert_eq!(rgba(H - 1, 0), BOTTOM_LEFT, "{reading:?}: bottom left");
            assert_eq!(rgba(H - 1, W - 1), bottom_right, "{reading:?}: the alpha");
            println!(
                "syphon: {reading:?} received the right way up, the half-covered corner {bottom_right:?}"
            );
        }
        drop(outlet);
        println!("syphon: a server is received by its label, copied into a texture of ours");
    }
}
