// SPDX-License-Identifier: AGPL-3.0-or-later

//! supersilvia's Spout peer, which CI runs against itself and against a peer built on the
//! Spout SDK, in separate processes on one Windows machine.
//!
//! ```text
//! spout_peer send    <sender> <width> <height> <seconds>
//! spout_peer receive <sender> <width> <height> <timeout-seconds>
//! ```
//!
//! The pattern, top row first, 8 bits a channel: pixel (x, y) is R = round(255 x / (w − 1)),
//! G = round(255 y / (h − 1)), B = 64, A = 255. `send` publishes it under `<sender>`, exactly
//! that name, about 30 times a second for that long through the app's own Spout sender and the
//! texture the renderer draws into, and exits 0, or 1 where it cannot send. `receive` waits for
//! the sender and reads its frames through the app's own receiver, the renderer's import and
//! its copy into a texture, and exits 0 once two frames in a row match within 2 a channel, or 1
//! at the timeout, saying what it saw. A command line it cannot read exits 2, as does any
//! machine but Windows.
//!
//! The device is made as the app makes it, on the adapter `render::adapter` picks, so
//! `SUPERSILVIA_SOFTWARE_GPU=1` and `SUPERSILVIA_ADAPTER` choose it; its LUID is printed first,
//! `adapter-luid <hex>`, `(HighPart << 32) | LowPart`, for the other peer to make its Direct3D
//! 11 device on, since a shared texture opens only on its sender's adapter.

use std::process::ExitCode;

fn main() -> ExitCode {
    #[cfg(target_os = "windows")]
    {
        peer::main()
    }
    #[cfg(not(target_os = "windows"))]
    {
        eprintln!("spout_peer: Spout is Windows' alone");
        ExitCode::from(2)
    }
}

#[cfg(target_os = "windows")]
mod peer {
    use std::process::ExitCode;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use supersilvia::graph::{NodeId, PortRef};
    use supersilvia::nodes::{Frame, Pixels};
    use supersilvia::platform::syphon::{Look, Server, servers};
    use supersilvia::render::adapter::Asked;
    use supersilvia::render::{FrameJob, Gpu, Renderer, SourceJob, dmabuf, readback};
    use supersilvia::video::syphon::Receiver;

    /// How far a channel may be from the pattern and still match.
    const TOLERANCE: u8 = 2;
    /// How often a frame is sent.
    const SEND_EVERY: Duration = Duration::from_millis(33);
    /// How often a receiver looks for a new frame.
    const LOOK_EVERY: Duration = Duration::from_millis(15);
    /// The node the received frame is copied for.
    const NODE: NodeId = NodeId(1);

    struct Args {
        send: bool,
        name: String,
        width: u32,
        height: u32,
        seconds: u64,
    }

    fn args() -> Result<Args, String> {
        let all: Vec<String> = std::env::args().skip(1).collect();
        let [mode, name, width, height, seconds] = all.as_slice() else {
            return Err(format!("five arguments, not {}", all.len()));
        };
        let send = match mode.as_str() {
            "send" => true,
            "receive" => false,
            other => return Err(format!("{other:?} is neither send nor receive")),
        };
        let size = |text: &str| {
            text.parse::<u32>()
                .ok()
                .filter(|n| (1..=16384).contains(n))
                .ok_or_else(|| format!("{text:?} is not a size"))
        };
        Ok(Args {
            send,
            name: name.clone(),
            width: size(width)?,
            height: size(height)?,
            seconds: seconds
                .parse()
                .ok()
                .filter(|&s| s > 0)
                .ok_or_else(|| format!("{seconds:?} is not a whole number of seconds"))?,
        })
    }

    pub fn main() -> ExitCode {
        env_logger::Builder::from_env(
            env_logger::Env::default().default_filter_or("warn,supersilvia=info"),
        )
        .init();
        let args = match args() {
            Ok(args) => args,
            Err(e) => {
                eprintln!(
                    "spout_peer: {e}\nusage: spout_peer send|receive <sender> <width> <height> <seconds>"
                );
                return ExitCode::from(2);
            }
        };
        let mode = if args.send { "send" } else { "receive" };
        let gpu = match Gpu::headless(&Asked::from_env()) {
            Ok(gpu) => gpu,
            Err(e) => {
                println!("spout_peer {mode}: no GPU: {e}");
                return ExitCode::from(1);
            }
        };
        let result = if args.send {
            send(&gpu, &args)
        } else {
            receive(&gpu, &args)
        };
        match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                println!("spout_peer {mode}: {e}");
                ExitCode::from(1)
            }
        }
    }

    /// The renderer's adapter, said, and its LUID on a line of its own for a script to read.
    fn say_adapter(gpu: &Gpu, mode: &str) -> Result<(), String> {
        let here = dmabuf::d3d12_here().ok_or("the device is not Direct3D 12")?;
        let info = gpu.adapter().get_info();
        println!(
            "spout_peer {mode}: rendering on {} ({:?}, {:?})",
            info.name, info.backend, info.device_type
        );
        println!("adapter-luid {:016x}", here.luid as u64);
        Ok(())
    }

    /// round(255 i / (n − 1)) in whole numbers, as the Spout SDK's peer rounds it.
    fn ramp(i: u32, n: u32) -> u8 {
        if n < 2 {
            return 0;
        }
        ((2 * 255 * i + (n - 1)) / (2 * (n - 1))) as u8
    }

    /// The pattern's pixel at (x, y), as R, G, B, A.
    fn expected(x: u32, y: u32, args: &Args) -> [u8; 4] {
        [ramp(x, args.width), ramp(y, args.height), 64, 255]
    }

    fn send(gpu: &Gpu, args: &Args) -> Result<(), String> {
        dmabuf::serve(gpu.device(), gpu.queue());
        say_adapter(gpu, "send")?;
        let (w, h) = (args.width, args.height);
        let server = Server::named_exactly(&args.name)?;
        let surface = server.surface(w, h)?;
        let texture = dmabuf::surface_target(gpu.device(), surface.raw(), w, h)?;
        let mut bgra = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let [r, g, b, a] = expected(x, y, args);
                bgra.extend_from_slice(&[b, g, r, a]);
            }
        }
        let start = Instant::now();
        let length = Duration::from_secs(args.seconds);
        let mut frames = 0u64;
        while start.elapsed() < length {
            gpu.queue().write_texture(
                texture.as_image_copy(),
                &bgra,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w * 4),
                    rows_per_image: None,
                },
                texture.size(),
            );
            let ticket = gpu.submit(std::iter::empty());
            gpu.wait(&ticket, Duration::from_secs(10))
                .map_err(|e| format!("the GPU did not finish a frame: {e}"))?;
            server.publish();
            if frames == 0 {
                let named = server.described().name;
                if named != args.name {
                    return Err(format!(
                        "the name {:?} is another sender's; Spout's list took {named:?}",
                        args.name
                    ));
                }
                let listed: Vec<String> = servers().into_iter().map(|s| s.name).collect();
                if !listed.contains(&args.name) {
                    return Err(format!(
                        "{:?} is not in Spout's list, which holds {listed:?}",
                        args.name
                    ));
                }
                println!(
                    "spout_peer send: sending {:?}, {w}x{h} B8G8R8A8_UNORM (DXGI format {}), for \
                     {} s; Spout lists {listed:?}",
                    args.name,
                    surface.pixel_format(),
                    args.seconds
                );
            }
            frames += 1;
            std::thread::sleep(SEND_EVERY);
        }
        println!("spout_peer send: sent {frames} frames");
        Ok(())
    }

    /// The first pixel of `rgba`, rows top first, that is not the pattern, or `None` where all
    /// are within [`TOLERANCE`].
    fn mismatch(rgba: &[u8], args: &Args) -> Option<String> {
        for y in 0..args.height {
            for x in 0..args.width {
                let i = ((y * args.width + x) * 4) as usize;
                let seen: [u8; 4] = rgba.get(i..i + 4)?.try_into().ok()?;
                let want = expected(x, y, args);
                if seen
                    .iter()
                    .zip(want)
                    .any(|(&s, w)| s.abs_diff(w) > TOLERANCE)
                {
                    return Some(format!(
                        "the first mismatch is at ({x}, {y}): RGBA {seen:?}, not {want:?}"
                    ));
                }
            }
        }
        None
    }

    /// What a received frame is, in a few words.
    fn described(frame: &Frame) -> String {
        match &frame.pixels {
            Pixels::D3d12(t) => format!(
                "{}x{}, a Direct3D 12 texture read as {:?}",
                frame.width, frame.height, t.layout
            ),
            _ => format!(
                "{}x{}, not a Direct3D 12 texture",
                frame.width, frame.height
            ),
        }
    }

    fn receive(gpu: &Gpu, args: &Args) -> Result<(), String> {
        let mut renderer = Renderer::new(gpu.clone())?;
        say_adapter(gpu, "receive")?;
        let deadline = Instant::now() + Duration::from_secs(args.seconds);
        let found = loop {
            if let Some(found) = servers().into_iter().find(|s| s.name == args.name) {
                break found;
            }
            if Instant::now() >= deadline {
                let listed: Vec<String> = servers().into_iter().map(|s| s.name).collect();
                return Err(format!(
                    "no sender {:?} appeared; Spout lists {listed:?}",
                    args.name
                ));
            }
            std::thread::sleep(LOOK_EVERY);
        };
        println!("spout_peer receive: found {:?}", found.label());
        let mut receiver = Receiver::new(&found.label(), Look::default());
        let mut last: Option<Arc<Frame>> = None;
        let mut matched = 0;
        let mut said = String::new();
        while Instant::now() < deadline {
            std::thread::sleep(LOOK_EVERY);
            let Some(frame) = receiver.latest() else {
                continue;
            };
            if last.as_ref().is_some_and(|l| Arc::ptr_eq(l, &frame)) {
                continue;
            }
            if last.is_none() {
                println!(
                    "spout_peer receive: the first frame is {}",
                    described(&frame)
                );
            }
            last = Some(Arc::clone(&frame));
            renderer.draw(&FrameJob {
                sources: vec![SourceJob::new(PortRef::new(NODE, "frame"), frame)],
                ..FrameJob::default()
            });
            gpu.device()
                .poll(wgpu::PollType::wait_indefinitely())
                .map_err(|e| format!("the GPU did not finish the copy: {e}"))?;
            let Some(texture) = renderer.texture_of(NODE) else {
                let why = renderer
                    .errors
                    .get(&NODE)
                    .cloned()
                    .unwrap_or_else(|| "nothing".to_owned());
                say_once(&mut said, &format!("the frame was not copied: {why}"));
                matched = 0;
                continue;
            };
            let what = if (texture.width(), texture.height()) == (args.width, args.height) {
                mismatch(&readback::read_texture(gpu, &texture), args)
            } else {
                Some(format!(
                    "the sender is {}x{}, not {}x{}",
                    texture.width(),
                    texture.height(),
                    args.width,
                    args.height
                ))
            };
            match what {
                None => {
                    matched += 1;
                    if matched >= 2 {
                        println!("spout_peer receive: two frames in a row match the pattern");
                        return Ok(());
                    }
                }
                Some(why) => {
                    matched = 0;
                    say_once(&mut said, &why);
                }
            }
        }
        Err(match (last.is_some(), receiver.error()) {
            (false, Some(why)) => format!("no frame arrived: {why}"),
            (false, None) => "no frame arrived".to_owned(),
            (true, _) if said.is_empty() => "only one frame matched".to_owned(),
            (true, _) => format!("no match: {said}"),
        })
    }

    /// Print `what` where it is not what was printed last.
    fn say_once(said: &mut String, what: &str) {
        if said != what {
            println!("spout_peer receive: {what}");
            what.clone_into(said);
        }
    }
}
