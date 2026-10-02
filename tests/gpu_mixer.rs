// SPDX-License-Identifier: AGPL-3.0-or-later

//! The renderer's mix and its viewer — the eight crossfades at the pixels that tell them apart, a deck of another shape, a
//! claim that allocates nothing, a mix that keeps its picture through a drag — and the blit
//! read back from an offscreen target: through egui_wgpu's own renderer, clip rect and all, for
//! the editor, and in a pass of its own for a picture window's surface. The fit, the rect that
//! hangs off the window, the corners and the flip each have a test, because each is where the
//! window's top-first rows could turn a picture upside down or round the wrong corners.
//!
//! The mix keeps GL's rows, bottom first, so its reads count `y` from the bottom. A blit's target is a window, whose row 0 is its top, so its reads count
//! `y` from the top.

#[path = "common/gpu.rs"]
mod gpu;

use gpu::{bytes_of, drain, job, link_all, module, solid, tick};
use std::sync::Arc;
use supersilvia::compile::wgsl::Sampler;
use supersilvia::graph::NodeId;
use supersilvia::mixer::Method;
use supersilvia::render::Fit;
use supersilvia::render::mixer::{Deck, MIX, Mixer};
use supersilvia::render::queue::Recording;
use supersilvia::render::ring::RING;
use supersilvia::render::shared::{self, Shared};
use supersilvia::render::viewer::{Viewer, Viewport};
use supersilvia::render::{
    FrameJob, Gpu, MixerJob, OutputJob, OutputMode, Picture, Published, Renderer, Texture,
};

const DRAW: OutputMode = OutputMode::Draw;
const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const BLACK: [u8; 3] = [0, 0, 0];
const WHITE: [u8; 3] = [255, 255, 255];

// ------------------------------------------------------------------------------ the helpers

/// A texture of `format` cleared to `color`: an Output's frame of one color.
fn cleared(
    gpu: &Gpu,
    size: (u32, u32),
    format: wgpu::TextureFormat,
    color: wgpu::Color,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("test target"),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    shared::clear(&mut encoder, &view, color);
    gpu.submit([encoder.finish()]);
    (texture, view)
}

fn color([r, g, b]: [u8; 3]) -> wgpu::Color {
    wgpu::Color {
        r: f64::from(r) / 255.0,
        g: f64::from(g) / 255.0,
        b: f64::from(b) / 255.0,
        a: 1.0,
    }
}

/// An `Rgba8Unorm` texture holding `rows`, first row first, each row one color per texel.
fn uploaded(gpu: &Gpu, rows: &[Vec<[u8; 3]>]) -> Picture {
    let (width, height) = (rows[0].len() as u32, rows.len() as u32);
    let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("test picture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let bytes: Vec<u8> = rows
        .iter()
        .flatten()
        .flat_map(|[r, g, b]| [*r, *g, *b, 255])
        .collect();
    gpu.queue().write_texture(
        texture.as_image_copy(),
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: None,
        },
        texture.size(),
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Picture {
        texture: Texture::new(texture, view),
        width,
        height,
        flip: false,
        sampler: Sampler::MirrorLinear,
        drawn_tick: None,
    }
}

/// `height` rows of `width` texels, the left `split` of each row `left` and the rest `right`.
fn columns(
    width: usize,
    height: usize,
    split: usize,
    left: [u8; 3],
    right: [u8; 3],
) -> Vec<Vec<[u8; 3]>> {
    let row: Vec<[u8; 3]> = (0..width)
        .map(|x| if x < split { left } else { right })
        .collect();
    vec![row; height]
}

/// `height` rows of `width` texels, the first `split` rows `first` and the rest `rest`.
fn bands(
    width: usize,
    height: usize,
    split: usize,
    first: [u8; 3],
    rest: [u8; 3],
) -> Vec<Vec<[u8; 3]>> {
    (0..height)
        .map(|y| vec![if y < split { first } else { rest }; width])
        .collect()
}

/// RGB of the texel at `(x, y)` of RGBA8 bytes `width` wide, `y` counting rows as they lie.
fn pixel(bytes: &[u8], width: u32, x: u32, y: u32) -> [u8; 3] {
    let at = ((y * width + x) * 4) as usize;
    [bytes[at], bytes[at + 1], bytes[at + 2]]
}

/// Near `want`, within a rounding either way.
fn near(got: [u8; 3], want: [u8; 3]) -> bool {
    got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 2)
}

/// A published set holding `picture` as the Output `node`'s frame.
fn holding(node: NodeId, picture: Picture) -> Arc<Published> {
    let mut published = Published::default();
    published.outputs.insert(node, picture);
    Arc::new(published)
}

// ------------------------------------------------------------------------------- the mixer

/// The mixer driven directly, one tick at a time, each waited for.
struct Bench {
    gpu: Gpu,
    shared: Shared,
    mixer: Mixer,
    tick: u64,
}

impl Bench {
    fn new() -> Self {
        let gpu = gpu::gpu();
        Self {
            shared: Shared::new(&gpu),
            mixer: Mixer::new(&gpu),
            tick: 0,
            gpu,
        }
    }

    /// One tick's mix of `a` and `b`, submitted and waited for: the latest mix, rows bottom
    /// first.
    fn mix(&mut self, job: &MixerJob, a: Option<Deck<'_>>, b: Option<Deck<'_>>) -> Vec<u8> {
        self.tick += 1;
        let mut recording = Recording::new(&self.gpu, "test mix");
        self.mixer
            .sync(&self.gpu, &self.shared, &mut recording, job, self.tick)
            .expect("the mix is made");
        self.mixer
            .draw(&self.gpu, &self.shared, &mut recording, job, a, b);
        if let Some(commands) = recording.finish() {
            let ticket = self.gpu.submit([commands]);
            self.mixer.submitted(&ticket);
        }
        drain(&self.gpu);
        bytes_of(&self.gpu, &self.mixer.texture().expect("the mix exists"))
    }
}

fn mix_job(balance: f32, method: Method, resolution: (u32, u32)) -> MixerJob {
    MixerJob {
        a: None,
        b: None,
        balance,
        method,
        resolution,
        blackout: false,
        freeze: false,
    }
}

/// The eight crossfades, each read at the pixels that tell them apart. Red is deck A and
/// blue is deck B, so every answer is one of two colors and a blend is unmistakable.
#[test]
fn the_mixer_crossfades_two_solid_decks() {
    let mut bench = Bench::new();
    let half = wgpu::TextureFormat::Rgba16Float;
    let (_a, a) = cleared(&bench.gpu, (64, 64), half, color(RED));
    let (_b, b) = cleared(&bench.gpu, (64, 64), half, color(BLUE));
    let deck = |view| {
        Some(Deck {
            view,
            width: 64,
            height: 64,
        })
    };
    let mut mix =
        |balance, method| bench.mix(&mix_job(balance, method, (64, 64)), deck(&a), deck(&b));
    let at = |t: &[u8], x, y| pixel(t, 64, x, y);

    // The fade's ends are hard, whatever the method.
    for method in Method::ALL {
        assert_eq!(at(&mix(-1.0, method), 32, 32), RED, "{method:?} at -1");
        assert_eq!(at(&mix(1.0, method), 32, 32), BLUE, "{method:?} at +1");
    }

    let [r, g, bl] = at(&mix(0.0, Method::Blend), 32, 32);
    assert!(
        g == 0 && (118..=137).contains(&r) && (118..=137).contains(&bl),
        "half and half: {r} {g} {bl}"
    );

    let t = mix(0.0, Method::HorizontalWipe);
    assert_eq!(at(&t, 8, 32), BLUE, "B wipes in from the left");
    assert_eq!(at(&t, 56, 32), RED);

    let t = mix(0.0, Method::VerticalWipe);
    assert_eq!(at(&t, 32, 8), BLUE, "B wipes in from the bottom");
    assert_eq!(at(&t, 32, 56), RED);

    let t = mix(0.0, Method::RadialWipe);
    assert_eq!(at(&t, 32, 32), BLUE, "B opens from the center");
    assert_eq!(at(&t, 1, 1), RED);

    // Red's luminance is 0.299: darker than a half-way fade, so dark-first shows B and
    // light-first still shows A.
    assert_eq!(at(&mix(0.0, Method::DarkFirst), 32, 32), BLUE);
    assert_eq!(at(&mix(0.0, Method::LightFirst), 32, 32), RED);

    // Odd cells show B below the wipe line and even cells show B above it, so at half way the
    // bottom-left cell (even) is A, the one beside it (odd) is B, and the top row is the other
    // way about.
    let t = mix(0.0, Method::Checkerboard);
    assert_eq!(at(&t, 4, 4), RED);
    assert_eq!(at(&t, 12, 4), BLUE, "the next cell along is odd");
    assert_eq!(at(&t, 4, 60), RED, "odd, but above the line");
    assert_eq!(at(&t, 12, 60), BLUE, "even, and above the line");

    // Even rows wipe right-to-left, odd rows left-to-right: the bottom row is even.
    let t = mix(0.0, Method::HorizontalLines);
    assert_eq!(at(&t, 4, 4), RED);
    assert_eq!(at(&t, 60, 4), BLUE);
    assert_eq!(at(&t, 4, 12), BLUE, "the next row up is odd");

    // An empty deck is black, and so is the mix of two empty decks.
    let job = mix_job(-1.0, Method::Blend, (64, 64));
    assert_eq!(at(&bench.mix(&job, None, deck(&b)), 32, 32), BLACK);
    let job = mix_job(0.0, Method::Blend, (64, 64));
    assert_eq!(at(&bench.mix(&job, None, None), 32, 32), BLACK);
}

/// A deck of another shape is scaled about its center to the mix's height, silvia's way: a
/// 1:2 deck is mirrored out to the edges of a square mix, so the mix always fills the
/// projector.
#[test]
fn a_deck_of_another_shape_fills_the_mix() {
    let mut bench = Bench::new();
    let (_tall, tall) = cleared(
        &bench.gpu,
        (32, 64),
        wgpu::TextureFormat::Rgba16Float,
        color(GREEN),
    );
    let deck = Some(Deck {
        view: &tall,
        width: 32,
        height: 64,
    });
    let mix = bench.mix(&mix_job(-1.0, Method::Blend, (64, 64)), deck, None);
    for x in [1, 16, 32, 48, 62] {
        assert_eq!(
            pixel(&mix, 64, x, 32),
            GREEN,
            "column {x} is the deck, not a bar"
        );
    }
}

/// Two empty decks are drawn black once and then not again, and the first picture on a deck
/// is drawn on the tick it arrives.
#[test]
fn two_empty_decks_are_drawn_once() {
    let mut bench = Bench::new();
    let job = mix_job(0.0, Method::Blend, (16, 16));
    assert_eq!(pixel(&bench.mix(&job, None, None), 16, 8, 8), BLACK);
    let black = bench.mixer.texture();
    for _ in 0..3 {
        bench.mix(&job, None, None);
        assert_eq!(bench.mixer.texture(), black, "a second black mix was drawn");
    }
    let (_red, red) = cleared(
        &bench.gpu,
        (16, 16),
        wgpu::TextureFormat::Rgba16Float,
        color(RED),
    );
    let deck = Some(Deck {
        view: &red,
        width: 16,
        height: 16,
    });
    let job = mix_job(-1.0, Method::Blend, (16, 16));
    assert_eq!(pixel(&bench.mix(&job, deck, None), 16, 8, 8), RED);
    assert_ne!(bench.mixer.texture(), black);
    assert_eq!(
        bench.mixer.dropped(),
        0,
        "a skip of two empty decks is not a drop"
    );
}

/// A mix larger than the GPU's largest texture is refused with a reason, and the mix stays at
/// the size it had rather than failing validation.
#[test]
fn a_mix_too_large_for_the_gpu_is_refused() {
    let mut bench = Bench::new();
    let job = mix_job(0.0, Method::Blend, (32, 16));
    bench.mix(&job, None, None);
    let largest = bench.gpu.device().limits().max_texture_dimension_2d;
    let mut recording = Recording::new(&bench.gpu, "too large");
    let err = bench
        .mixer
        .sync(
            &bench.gpu,
            &bench.shared,
            &mut recording,
            &mix_job(0.0, Method::Blend, (largest + 1, 16)),
            99,
        )
        .expect_err("refused");
    assert!(err.contains(&largest.to_string()), "{err}");
    assert!(!recording.used(), "nothing was recorded for it");
    assert_eq!(bench.mixer.size(), Some((32, 16)));
}

/// Going on air is the worst moment for a flash, so a claim reallocates nothing: the Output's
/// targets and the mix's are the same before and after, and the mix shows the deck on the
/// first tick. While the deck's graph relinks the mix keeps showing it, and the tick that
/// changes the mix's shape draws it.
#[test]
fn a_claim_reallocates_nothing_and_the_mix_follows_the_deck() {
    let gpu = gpu::gpu();
    let mut r = Renderer::new(gpu.clone()).expect("renderer");
    let node = NodeId(1);
    let white = solid(1.0, 1.0, 1.0);
    let outputs = |send, mode| vec![job(node, (64, 64), &white, send, mode, vec![])];
    let frame = |a: Option<NodeId>, resolution, outputs: Vec<OutputJob>| FrameJob {
        mixer: MixerJob {
            a,
            b: None,
            balance: -1.0,
            method: Method::Blend,
            resolution,
            blackout: false,
            freeze: false,
        },
        ..tick(0.0, outputs)
    };
    link_all(&mut r, &outputs);

    // On air and then off, drained after every tick, so both rings hold what a tick at this
    // size needs and what either needs on the claim's tick is down to the claim, not to how far
    // behind the GPU happens to be.
    for a in [Some(node), Some(node), Some(node), None, None, None] {
        r.draw(&frame(a, (64, 64), outputs(false, DRAW)));
        drain(&gpu);
    }
    assert_eq!(r.errors.get(&node), None);
    assert_eq!(r.mixer_error, None);
    let texture = r.texture_of(node).expect("drawn");
    assert_eq!(gpu::rgba_of(&gpu, &texture)[32 * 64 + 32], [255; 4]);
    let mix = r
        .mixer_texture()
        .expect("the mix exists from the first tick");
    assert_eq!(
        pixel(&bytes_of(&gpu, &mix), 64, 32, 32),
        BLACK,
        "nothing is on air"
    );

    // On air: the same targets, and the mix has the deck this tick.
    let (targets, mixes) = (r.targets_of(node), r.mixer_targets());
    r.draw(&frame(Some(node), (64, 64), outputs(false, DRAW)));
    assert_eq!(
        r.targets_of(node),
        targets,
        "the claim reallocated the Output's targets"
    );
    assert_eq!(r.mixer_targets(), mixes, "the claim reallocated the mix");
    drain(&gpu);
    let mix = r.mixer_texture().expect("a mix");
    assert_eq!(
        pixel(&bytes_of(&gpu, &mix), 64, 32, 32),
        WHITE,
        "deck A is on the projector"
    );

    // The deck's graph is edited: a new module is sent. The mix does not blink.
    let edited = module(
        &[],
        &[],
        "
// edited
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    return vec4f(1.0, 1.0, 1.0, 1.0);
}
",
    );
    let relinked = |send| vec![job(node, (64, 64), &edited, send, DRAW, vec![])];
    r.draw(&frame(Some(node), (64, 64), relinked(true)));
    drain(&gpu);
    let mix = r.mixer_texture().expect("a mix");
    assert_eq!(
        pixel(&bytes_of(&gpu, &mix), 64, 32, 32),
        WHITE,
        "still on air while it links"
    );
    for _ in 0..3 {
        r.draw(&frame(Some(node), (64, 64), relinked(false)));
    }
    drain(&gpu);
    assert_eq!(r.errors.get(&node), None);
    let mix = r.mixer_texture().expect("a mix");
    assert_eq!(pixel(&bytes_of(&gpu, &mix), 64, 32, 32), WHITE);

    // The mix changes shape. The tick that resizes it also draws it, so it is never blank.
    r.draw(&frame(Some(node), (32, 96), relinked(false)));
    assert_eq!(r.mixer_size(), Some((32, 96)));
    drain(&gpu);
    let mix = bytes_of(&gpu, &r.mixer_texture().expect("a mix"));
    assert_eq!(pixel(&mix, 32, 16, 48), WHITE);
    assert_eq!(
        pixel(&mix, 32, 1, 1),
        WHITE,
        "a wider deck is cropped, not barred"
    );
}

/// A deck whose red is the tick's time, so a mix a tick behind reads another value.
fn timed() -> Arc<supersilvia::compile::Shader> {
    module(
        &[],
        &[],
        "
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    return vec4f(fract(u.u_time), 0.0, 0.0, 1.0);
}
",
    )
}

/// A mix a viewer holds is never drawn into: its picture is the one it was when published,
/// however many ticks go by, while the latest mix moves on.
#[test]
fn a_mix_a_viewer_holds_is_never_drawn_into() {
    let gpu = gpu::gpu();
    let mut r = Renderer::new(gpu.clone()).expect("renderer");
    let node = NodeId(1);
    let shader = timed();
    let outputs = |send, mode| vec![job(node, (32, 32), &shader, send, mode, vec![])];
    let frame = |time, outputs| FrameJob {
        mixer: MixerJob {
            a: Some(node),
            resolution: (32, 32),
            ..MixerJob::default()
        },
        ..tick(time, outputs)
    };
    link_all(&mut r, &outputs);
    r.draw(&frame(0.25, outputs(false, DRAW)));
    drain(&gpu);
    let held = r.publish();
    let picture = held
        .mixer
        .clone()
        .expect("the mix is published once finished");
    assert!(!picture.flip, "the mix's first row is its bottom");
    let before = bytes_of(&gpu, picture.texture.texture());
    assert_eq!(pixel(&before, 32, 16, 16)[0], 64);
    for i in 0..8 {
        r.draw(&frame(0.5 + i as f32 * 0.01, outputs(false, DRAW)));
        drain(&gpu);
        drop(r.publish());
    }
    assert_eq!(
        bytes_of(&gpu, picture.texture.texture()),
        before,
        "a held mix was drawn into"
    );
    let latest = bytes_of(&gpu, &r.mixer_texture().expect("a mix"));
    assert_ne!(pixel(&latest, 32, 16, 16)[0], 64, "the mix moved on");
    assert!(r.mixer_targets().len() <= RING);
    drop(held);
}

/// The mix keeps its picture while its size follows a drag: a new size every tick, two
/// viewers' frames held, and never a black mix, never more than one skip a tick, never more
/// targets than the ring allows.
#[test]
fn the_mix_keeps_its_picture_while_its_size_follows_a_drag() {
    let gpu = gpu::gpu();
    let mut r = Renderer::new(gpu.clone()).expect("renderer");
    let node = NodeId(1);
    let shader = timed();
    let outputs = |send, mode| vec![job(node, (320, 180), &shader, send, mode, vec![])];
    let frame = |time, mix: (u32, u32)| FrameJob {
        mixer: MixerJob {
            a: Some(node),
            resolution: mix,
            ..MixerJob::default()
        },
        ..tick(time, outputs(false, DRAW))
    };
    link_all(&mut r, &outputs);
    r.draw(&frame(0.25, (400, 225)));
    drain(&gpu);
    drop(r.publish());
    let mut held: std::collections::VecDeque<Published> = std::collections::VecDeque::new();
    let before = r.mixer_drops();
    let (mut most, mut blank) = (0, Vec::new());
    for i in 0..60u32 {
        let time = (100 + i) as f32 * 0.01 + 0.005;
        r.draw(&frame(time, (400 + i * 3, 225)));
        held.push_back(r.publish());
        while held.len() > 2 {
            held.pop_front();
        }
        most = most.max(r.mixer_targets().len());
        if i % 10 == 9 {
            drain(&gpu);
            let (w, h) = r.mixer_size().expect("a mix");
            let mix = bytes_of(&gpu, &r.mixer_texture().expect("a mix"));
            let red = pixel(&mix, w, w / 2, h / 2)[0];
            if red == 0 {
                blank.push(i);
            }
        }
    }
    let dropped = r.mixer_drops() - before;
    drop(held);
    for i in 0..10 {
        r.draw(&frame(1.0 + i as f32 * 0.01, (444, 225)));
        drop(r.publish());
    }
    drain(&gpu);
    let last = r.publish().mixer.expect("the mix has a picture");
    assert_eq!((last.width, last.height), (444, 225));
    assert!(
        blank.is_empty(),
        "the mix showed black during the drag: {blank:?}"
    );
    assert!(dropped <= 60, "at most one skip per tick: {dropped}");
    assert!(most <= RING, "the mix's ring passed its limit: {most}");
}

// ------------------------------------------------------------------------------ the viewer

/// A window `side` pixels square at `pixels_per_point`, cleared red and painted by egui_wgpu's
/// own renderer with `callbacks`, each under its clip rect in points: RGBA8, rows top first.
fn in_egui(
    gpu: &Gpu,
    side: u32,
    pixels_per_point: f32,
    callbacks: Vec<(egui::Rect, egui::PaintCallback)>,
) -> Vec<u8> {
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut renderer =
        egui_wgpu::Renderer::new(gpu.device(), format, egui_wgpu::RendererOptions::default());
    let primitives: Vec<egui::epaint::ClippedPrimitive> = callbacks
        .into_iter()
        .map(|(clip_rect, callback)| egui::epaint::ClippedPrimitive {
            clip_rect,
            primitive: egui::epaint::Primitive::Callback(callback),
        })
        .collect();
    let screen = egui_wgpu::ScreenDescriptor {
        size_in_pixels: [side, side],
        pixels_per_point,
    };
    let (target, view) = cleared(gpu, (side, side), format, color(RED));
    let mut encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    let prepared = renderer.update_buffers(
        gpu.device(),
        gpu.queue(),
        &mut encoder,
        &primitives,
        &screen,
    );
    {
        let mut pass =
            shared::begin(&mut encoder, &view, wgpu::LoadOp::Load, "egui").forget_lifetime();
        renderer.render(&mut pass, &primitives, &screen);
    }
    gpu.submit(prepared.into_iter().chain([encoder.finish()]));
    drain(gpu);
    bytes_of(gpu, &target)
}

/// `picture` blitted into the whole of a `side`-pixel square target of `format` cleared red,
/// in a pass of its own, as a picture window's surface is: RGB, rows top first.
fn on_surface(
    gpu: &Gpu,
    format: wgpu::TextureFormat,
    side: u32,
    picture: &Picture,
    fit: Fit,
) -> Vec<u8> {
    let viewer = Viewer::new(gpu, format).expect("the blit pipeline");
    let (target, view) = cleared(gpu, (side, side), format, color(RED));
    let mut encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    viewer.show(
        &mut encoder,
        &view,
        (side, side),
        picture,
        Viewport::whole((side, side)),
        fit,
        0.0,
    );
    gpu.submit([encoder.finish()]);
    drain(gpu);
    let mut bytes = bytes_of(gpu, &target);
    if format == wgpu::TextureFormat::Bgra8Unorm {
        for texel in bytes.as_chunks_mut::<4>().0 {
            texel.swap(0, 2);
        }
    }
    bytes
}

fn square(side: f32) -> egui::Rect {
    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(side, side))
}

/// A node's blit fits the rect it was given the way the caller asked, egui's clip rect cuts
/// it off rather than refitting it, and a radius rounds the rect's two bottom corners — the
/// window's bottom, although the window's row 0 is its top.
#[test]
fn a_blit_fits_its_rect_and_the_clip_cuts_it_rather_than_shrinking_it() {
    const SIDE: u32 = 128;
    let gpu = gpu::gpu();
    let viewer = Arc::new(Viewer::new(&gpu, wgpu::TextureFormat::Rgba8Unorm).expect("viewer"));
    let node = NodeId(1);
    let published = holding(node, uploaded(&gpu, &columns(64, 32, 64, GREEN, GREEN)));
    let whole = square(SIDE as f32);
    let blit = |clip: egui::Rect, fit: Fit, corner: f32| {
        let callback = viewer.node_callback(&published, whole, node, None, fit, corner);
        in_egui(&gpu, SIDE, 1.0, vec![(clip, callback)])
    };
    let green = |px: &[u8], x: u32, y: u32| near(pixel(px, SIDE, x, y), GREEN);

    // A 2:1 picture covering a square fills every row: the crop is at the sides.
    let covered = blit(whole, Fit::Cover, 0.0);
    for row in [2, 8, 24, 40, 60, 70, 90, 104, 120, 125] {
        assert!(
            green(&covered, SIDE / 2, row),
            "row {row} is a bar, not a cover"
        );
    }

    // Letterboxed, it is half the height, centered: the outer quarter at each end is the
    // clear color.
    let letterboxed = blit(whole, Fit::Letterbox, 0.0);
    for row in [40, 60, 70, 90] {
        assert!(
            green(&letterboxed, SIDE / 2, row),
            "row {row} lost the picture"
        );
    }
    for row in [8, 24, 104, 120] {
        assert!(
            !green(&letterboxed, SIDE / 2, row),
            "row {row} is picture: a cover"
        );
    }

    // The bottom half clipped away: the top half is still the picture, nothing refitted.
    let top_half = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(128.0, 64.0));
    let clipped = blit(top_half, Fit::Cover, 0.0);
    for row in [7, 23, 37, 57] {
        assert!(
            green(&clipped, SIDE / 2, row),
            "row {row} lost the picture under the clip"
        );
    }
    for row in [67, 87, 103, 119] {
        assert_eq!(
            pixel(&clipped, SIDE, SIDE / 2, row),
            RED,
            "row {row} was outside the clip"
        );
    }

    // A radius rounds the two bottom corners and nothing else.
    let rounded = blit(whole, Fit::Cover, 32.0);
    let bottom = SIDE - 3;
    assert_eq!(
        pixel(&rounded, SIDE, 2, bottom),
        RED,
        "bottom-left corner was not rounded off"
    );
    assert_eq!(
        pixel(&rounded, SIDE, SIDE - 3, bottom),
        RED,
        "bottom-right corner was not rounded off"
    );
    assert!(
        green(&rounded, SIDE / 2, bottom),
        "the bottom edge between the corners is picture"
    );
    assert!(
        green(&rounded, 2, SIDE - 1 - 40),
        "the side above the arc is picture"
    );
    assert!(green(&rounded, 2, 2), "the top-left corner is square");
    assert!(
        green(&rounded, SIDE - 3, 2),
        "the top-right corner is square"
    );

    // A node with nothing published draws nothing.
    let nothing = viewer.node_callback(&published, whole, NodeId(9), None, Fit::Cover, 0.0);
    let empty = in_egui(&gpu, SIDE, 1.0, vec![(whole, nothing)]);
    assert_eq!(pixel(&empty, SIDE, SIDE / 2, SIDE / 2), RED);
}

/// The corner is in points and the rect is in points: at two pixels a point both scale, so a
/// 64-point body with a 16-point corner is the 128-pixel blit with a 32-pixel one.
#[test]
fn a_blit_at_two_pixels_a_point_scales_its_rect_and_its_corner() {
    const SIDE: u32 = 128;
    let gpu = gpu::gpu();
    let viewer = Arc::new(Viewer::new(&gpu, wgpu::TextureFormat::Rgba8Unorm).expect("viewer"));
    let node = NodeId(1);
    let published = holding(node, uploaded(&gpu, &columns(64, 32, 64, GREEN, GREEN)));
    let callback = viewer.node_callback(&published, square(64.0), node, None, Fit::Cover, 16.0);
    let rounded = in_egui(&gpu, SIDE, 2.0, vec![(square(64.0), callback)]);
    let green = |x: u32, y: u32| near(pixel(&rounded, SIDE, x, y), GREEN);
    assert!(
        !green(2, SIDE - 3) && !green(SIDE - 3, SIDE - 3),
        "the bottom corners are round"
    );
    assert!(green(SIDE / 2, SIDE - 3) && green(2, SIDE - 41) && green(2, 2) && green(SIDE - 3, 2));
}

/// A rect hanging off the window keeps its size: what is on screen is the part of the whole
/// fit that lands there, never the whole picture squeezed into the visible part — which is
/// what egui_wgpu's clamped viewport would draw.
#[test]
fn a_rect_off_the_window_is_cut_off_rather_than_squeezed() {
    const SIDE: u32 = 128;
    let gpu = gpu::gpu();
    let viewer = Arc::new(Viewer::new(&gpu, wgpu::TextureFormat::Rgba8Unorm).expect("viewer"));
    let window = square(SIDE as f32);

    // Off the left edge by half: the visible half is the picture's right half, blue.
    let node = NodeId(1);
    let published = holding(node, uploaded(&gpu, &columns(64, 32, 32, GREEN, BLUE)));
    let rect = egui::Rect::from_min_size(egui::pos2(-64.0, 0.0), egui::vec2(128.0, 128.0));
    let callback = viewer.node_callback(&published, rect, node, None, Fit::Letterbox, 0.0);
    let left = in_egui(&gpu, SIDE, 1.0, vec![(window, callback)]);
    for x in [2, 32, 62] {
        assert!(
            near(pixel(&left, SIDE, x, 64), BLUE),
            "column {x} is not the right half"
        );
    }
    assert_eq!(
        pixel(&left, SIDE, 96, 64),
        RED,
        "the rect ends at the window's middle"
    );

    // Off the top by half: the visible half is the picture's bottom, the first rows of an
    // Output's frame.
    let published = holding(node, uploaded(&gpu, &bands(64, 64, 32, BLUE, GREEN)));
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, -64.0), egui::vec2(128.0, 128.0));
    let callback = viewer.node_callback(&published, rect, node, None, Fit::Letterbox, 0.0);
    let top = in_egui(&gpu, SIDE, 1.0, vec![(window, callback)]);
    for y in [2, 32, 62] {
        assert!(
            near(pixel(&top, SIDE, 64, y), BLUE),
            "row {y} is not the picture's bottom"
        );
    }
    assert_eq!(
        pixel(&top, SIDE, 64, 96),
        RED,
        "the rect ends at the window's middle"
    );

    // A rect far larger than the largest texture, around the window: the viewport a deep zoom
    // would ask for is refused by wgpu, and the blit draws the picture's middle regardless.
    // Nearest, since the window spans a hundredth of a texel either side of the split.
    let split = Picture {
        sampler: Sampler::MirrorNearest,
        ..uploaded(&gpu, &columns(64, 64, 32, GREEN, BLUE))
    };
    let published = holding(node, split);
    let huge = egui::Rect::from_center_size(egui::pos2(64.0, 64.0), egui::vec2(40_000.0, 40_000.0));
    let callback = viewer.node_callback(&published, huge, node, None, Fit::Cover, 0.0);
    let zoomed = in_egui(&gpu, SIDE, 1.0, vec![(window, callback)]);
    assert!(
        near(pixel(&zoomed, SIDE, 32, 64), GREEN),
        "left of the middle is the left half"
    );
    assert!(
        near(pixel(&zoomed, SIDE, 96, 64), BLUE),
        "right of the middle is the right half"
    );
}

/// The top of the picture is the top of the window, both for an Output's frame, whose first
/// row is its bottom, and for an upload, whose first row is its top and which says so — in
/// egui and on a surface of the other byte order.
#[test]
fn the_top_of_a_picture_is_the_top_of_the_window() {
    const SIDE: u32 = 64;
    let gpu = gpu::gpu();
    let node = NodeId(1);
    let output = uploaded(&gpu, &bands(32, 32, 16, BLUE, GREEN));
    let upload = Picture {
        flip: true,
        ..uploaded(&gpu, &bands(32, 32, 16, GREEN, BLUE))
    };
    let viewer = Arc::new(Viewer::new(&gpu, wgpu::TextureFormat::Rgba8Unorm).expect("viewer"));
    for (what, picture) in [("an Output's frame", output), ("an upload", upload)] {
        let published = holding(node, picture.clone());
        let callback =
            viewer.node_callback(&published, square(SIDE as f32), node, None, Fit::Cover, 0.0);
        let egui = in_egui(&gpu, SIDE, 1.0, vec![(square(SIDE as f32), callback)]);
        let surface = on_surface(
            &gpu,
            wgpu::TextureFormat::Bgra8Unorm,
            SIDE,
            &picture,
            Fit::Cover,
        );
        for (place, px) in [("egui", &egui), ("a surface", &surface)] {
            assert!(
                near(pixel(px, SIDE, 32, 4), GREEN),
                "{what} in {place}: the top is not green"
            );
            assert!(
                near(pixel(px, SIDE, 32, SIDE - 4), BLUE),
                "{what} in {place}: the bottom is not blue"
            );
        }
    }
}

/// A picture is read through the sampler it names: a nearest source is blitted as hard
/// squares, and the same source through a linear sampler is not.
#[test]
fn a_nearest_source_is_blitted_nearest() {
    let gpu = gpu::gpu();
    let checker = vec![vec![BLACK, WHITE], vec![WHITE, BLACK]];
    let nearest = Picture {
        sampler: Sampler::MirrorNearest,
        ..uploaded(&gpu, &checker)
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let hard = on_surface(&gpu, format, 16, &nearest, Fit::Cover);
    for texel in hard.as_chunks::<4>().0 {
        assert!(
            texel[..3] == [0, 0, 0] || texel[..3] == [255, 255, 255],
            "a nearest blit blended: {texel:?}"
        );
    }
    let linear = Picture {
        sampler: Sampler::MirrorLinear,
        ..nearest
    };
    let soft = on_surface(&gpu, format, 16, &linear, Fit::Cover);
    assert!(
        soft.as_chunks::<4>()
            .0
            .iter()
            .any(|t| (32..224).contains(&t[0])),
        "a linear blit has no grey between the squares"
    );
}

/// The background covers its rect: a 2:1 mix into a square target fills every row, cropped at
/// the sides, where the letterbox the preview uses leaves bars.
#[test]
fn the_background_covers_the_rect_where_the_preview_letterboxes() {
    const SIDE: u32 = 64;
    let gpu = gpu::gpu();
    let mut r = Renderer::new(gpu.clone()).expect("renderer");
    let node = NodeId(1);
    let white = solid(1.0, 1.0, 1.0);
    let outputs = |send, mode| vec![job(node, (64, 32), &white, send, mode, vec![])];
    link_all(&mut r, &outputs);
    let frame = FrameJob {
        mixer: MixerJob {
            a: Some(node),
            resolution: (64, 32),
            ..MixerJob::default()
        },
        ..tick(0.0, outputs(false, DRAW))
    };
    r.draw(&frame);
    r.draw(&frame);
    drain(&gpu);
    let published = Arc::new(r.publish());
    let mix = published.mixer.clone().expect("the mix is published");
    assert_eq!((mix.width, mix.height), (64, 32));

    let viewer = Arc::new(Viewer::new(&gpu, wgpu::TextureFormat::Rgba8Unorm).expect("viewer"));
    let blit = |fit| {
        let callback = viewer.mixer_callback(&published, square(SIDE as f32), fit);
        in_egui(&gpu, SIDE, 1.0, vec![(square(SIDE as f32), callback)])
    };
    let white = |px: &[u8], row| pixel(px, SIDE, SIDE / 2, row) == WHITE;
    let letterboxed = blit(Fit::Letterbox);
    assert!(white(&letterboxed, 32), "the picture is in the middle");
    assert!(!white(&letterboxed, 4), "and a bar is above and below it");
    let covered = blit(Fit::Cover);
    for row in [2, 16, 32, 48, 61] {
        assert!(
            white(&covered, row),
            "row {row} is a bar: the background did not cover"
        );
    }

    // The picture window's path draws the same mix the same way.
    let (target, view) = cleared(
        &gpu,
        (SIDE, SIDE),
        wgpu::TextureFormat::Rgba8Unorm,
        color(BLACK),
    );
    let mut encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    viewer.show_mixer(
        &mut encoder,
        &view,
        (SIDE, SIDE),
        &published,
        Viewport::whole((SIDE, SIDE)),
        Fit::Letterbox,
    );
    gpu.submit([encoder.finish()]);
    drain(&gpu);
    let window = bytes_of(&gpu, &target);
    assert!(white(&window, 32) && !white(&window, 4));
    drop(published);
}

// ---------------------------------------------------------------------- the mix's methods

/// Every crossfade method has its branch in the mix. The mix and the blit reach Metal and
/// Vulkan through `tests/shader_targets.rs`, with the renderer's other stages.
#[test]
fn every_crossfade_method_has_a_branch_in_the_mix() {
    for method in Method::ALL {
        assert!(
            MIX.contains(&format!("method == {}", method.index())) || method == Method::Blend,
            "{method:?} has no branch in the mix"
        );
    }
}

/// What a viewer is handed of the mix now: the newest finished mix, or the black Blackout
/// puts in its place.
fn shown_mix(bench: &mut Bench) -> Picture {
    bench.mixer.poll_frames(bench.gpu.completed());
    let mut out = Published::default();
    bench.mixer.publish(&mut out);
    out.mixer.expect("a mix is published")
}

/// **Blackout and Freeze are in the mix, before every viewer**, so the panel, the canvas, a
/// picture window, NDI and Syphon all show the same held picture. Freeze draws no new mix,
/// so the last one goes on being shown whatever the decks do; Blackout publishes black at the
/// mix's own size in its place; letting Blackout go while Freeze holds shows the frozen frame
/// again, stamped newer than the black, so an outlet that sends only what is newer sends it.
#[test]
fn freeze_holds_the_last_mix_and_blackout_shows_black_at_its_size() {
    let mut bench = Bench::new();
    let half = wgpu::TextureFormat::Rgba16Float;
    let (_a, red) = cleared(&bench.gpu, (64, 32), half, color(RED));
    let (_b, blue) = cleared(&bench.gpu, (64, 32), half, color(BLUE));
    let deck = |view| {
        Some(Deck {
            view,
            width: 64,
            height: 32,
        })
    };
    let size = (64, 32);
    let job = |blackout, freeze| MixerJob {
        blackout,
        freeze,
        ..mix_job(-1.0, Method::Blend, size)
    };
    let centre = |gpu: &Gpu, picture: &Picture| {
        let bytes = bytes_of(gpu, picture.texture.texture());
        if picture.texture.texture().width() == 1 {
            pixel(&bytes, 1, 0, 0)
        } else {
            pixel(&bytes, size.0, size.0 / 2, size.1 / 2)
        }
    };

    let t = bench.mix(&job(false, false), deck(&red), None);
    assert_eq!(pixel(&t, 64, 32, 16), RED, "deck A, red");

    let t = bench.mix(&job(false, true), deck(&blue), None);
    assert_eq!(
        pixel(&t, 64, 32, 16),
        RED,
        "frozen: the blue deck draws no mix"
    );
    let frozen = shown_mix(&mut bench);
    assert_eq!(centre(&bench.gpu, &frozen), RED, "and red is what is shown");

    bench.mix(&job(true, true), deck(&blue), None);
    let dark = shown_mix(&mut bench);
    assert_eq!(centre(&bench.gpu, &dark), BLACK, "Blackout: black");
    assert_eq!(
        (dark.width, dark.height),
        size,
        "at the mix's own size, so a window or a sender keeps its shape"
    );
    let again = shown_mix(&mut bench);
    assert_eq!(
        again.drawn_tick, dark.drawn_tick,
        "the same black, so an outlet sends it once"
    );

    bench.mix(&job(false, true), deck(&blue), None);
    let back = shown_mix(&mut bench);
    assert_eq!(
        centre(&bench.gpu, &back),
        RED,
        "Blackout let go: the frozen frame"
    );
    assert!(
        back.drawn_tick > dark.drawn_tick,
        "stamped newer than the black: {:?} after {:?}",
        back.drawn_tick,
        dark.drawn_tick
    );

    let t = bench.mix(&job(false, false), deck(&blue), None);
    assert_eq!(
        pixel(&t, 64, 32, 16),
        BLUE,
        "both let go: the mix as it is now"
    );
}
