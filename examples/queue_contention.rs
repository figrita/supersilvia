// SPDX-License-Identifier: AGPL-3.0-or-later

//! Does the editor starve behind the synth when both share one wgpu queue?
//!
//! ```sh
//! cargo build --release --example queue_contention
//! target/release/examples/queue_contention            # headless
//! target/release/examples/queue_contention --window   # an eframe window at the display's rate
//! ```
//!
//! Step 0b of `proposals/wgpu.md`, whose one-queue section holds the numbers. On the integrated
//! GPU, `render::adapter::Asked::integrated`, so never a discrete GPU.
//!
//! **The synth** is a thread ticking at 100 Hz from a deadline of its own, as `synth/`'s does,
//! under the one-tick bound: a tick first waits for the previous tick's last submission, then
//! submits eight fullscreen passes into 1280x720 `Rgba16Float` targets, one submission per
//! pass, as the renderer will submit one per Output. The passes are a loop of transcendental
//! math whose iteration count is calibrated on the GPU, so a tick costs a chosen number of
//! milliseconds: half the interval, nine tenths of it, or half as much again as fits, which is
//! a synth the GPU cannot keep up with and which therefore keeps it busy all the time.
//!
//! **The editor** is one pass over a 3440x1440 `Rgba8Unorm` target calibrated to 2.9 or
//! 4.8 ms — the two paint costs decisions.md's low-priority measurement used on GL, so the
//! figures compare. Headless, it is a second thread submitting at 100 Hz deadlines and waiting
//! for its own submission; with `--window` it is submitted from an eframe `ui` on wgpu at
//! the display's own vsync, sharing the device through `WgpuSetup::Existing`, with a watcher
//! thread waiting on each submission. Either way the figure is **submit to done**, the time
//! between handing the editor's work to the queue and the GPU finishing it; a frame whose work
//! is not done inside the 10 ms interval is an overrun. The window adds the interval between
//! frames as eframe produced them.
//!
//! **The synth runs four ways**: on the editor's own device and queue (Plan A); on the same
//! queue **throttled**, as the wgpu renderer's skeleton submits — each pass through
//! `render::queue::Throttle`, which submits the next only once at most
//! `render::QUEUED_AHEAD` (one) of its earlier submissions is still on the GPU; on a
//! second device from the same adapter at the driver's default priority; and on a second
//! device whose queue asks `VK_QUEUE_GLOBAL_PRIORITY_LOW` (Plan B, as the proposal describes
//! it: wgpu-hal's `open_with_callback` with `VkDeviceQueueGlobalPriorityCreateInfoKHR` in the
//! queue's chain). `--only one,throttled,two,low` runs the named ones alone.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};
use supersilvia::render::QUEUED_AHEAD;
use supersilvia::render::adapter;
use supersilvia::render::queue::{Recording, Throttle};

/// The display interval both loops keep headless, and the editor's budget.
const INTERVAL: Duration = Duration::from_millis(10);
/// Passes a synth tick submits, each a submission of its own.
const OUTPUTS: usize = 8;
/// One synth pass's target.
const OUTPUT_SIZE: (u32, u32) = (1280, 720);
/// The editor's target: a 3440×1440 ultrawide.
const EDITOR_SIZE: (u32, u32) = (3440, 1440);
/// The editor's two paint costs, in ms.
const EDITOR_MS: [f64; 2] = [2.9, 4.8];
/// A synth tick's GPU cost as a fraction of the interval, beside none at all.
const LOADS: [f64; 3] = [0.5, 0.9, 1.5];
const WARM: Duration = Duration::from_secs(1);
const RECORD: Duration = Duration::from_secs(6);
const RUNS: usize = 3;
const WAIT: Duration = Duration::from_secs(30);

const SHADER: &str = r"
struct Params { iters: u32, seed: f32, pad0: f32, pad1: f32 };
@group(0) @binding(0) var<uniform> params: Params;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    var v = pos.xy * 0.001 + vec2<f32>(params.seed);
    for (var i = 0u; i < params.iters; i++) {
        v = vec2<f32>(sin(v.x * 1.7 + v.y), cos(v.y * 1.3 - v.x));
    }
    return vec4<f32>(v * 0.5 + 0.5, 0.5, 1.0);
}
";

/// Where the synth's passes go.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Plan {
    /// The editor's own device and queue.
    OneQueue,
    /// The editor's own queue, each submission through the renderer's throttle.
    OneQueueThrottled,
    /// A second device from the same adapter, at the driver's default priority.
    TwoDevices,
    /// A second device whose queue is created at `VK_QUEUE_GLOBAL_PRIORITY_LOW`.
    TwoDevicesLow,
}

impl Plan {
    fn name(self) -> &'static str {
        match self {
            Plan::OneQueue => "one queue",
            Plan::OneQueueThrottled => "one queue, throttled",
            Plan::TwoDevices => "two devices",
            Plan::TwoDevicesLow => "two devices, synth low",
        }
    }
}

/// A device with its queue.
#[derive(Clone)]
struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

fn descriptor() -> wgpu::DeviceDescriptor<'static> {
    wgpu::DeviceDescriptor {
        label: Some("queue_contention"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
    }
}

fn open(adapter: &wgpu::Adapter) -> Gpu {
    let (device, queue) = adapter::block_on(adapter.request_device(&descriptor()))
        .expect("the adapter opens a device");
    Gpu { device, queue }
}

/// A device whose one queue asks the driver for low global priority.
#[cfg(target_os = "linux")]
fn open_low(adapter: &wgpu::Adapter) -> Result<Gpu, String> {
    use std::ffi::c_void;

    /// `VkDeviceQueueGlobalPriorityCreateInfoKHR`, laid out as Vulkan declares it.
    #[repr(C)]
    struct QueuePriority {
        s_type: i32,
        p_next: *const c_void,
        global_priority: i32,
    }
    const STRUCTURE_TYPE: i32 = 1_000_174_000;
    const LOW: i32 = 128;
    const EXTENSION: &std::ffi::CStr = c"VK_KHR_global_priority";

    let priority = QueuePriority {
        s_type: STRUCTURE_TYPE,
        p_next: std::ptr::null(),
        global_priority: LOW,
    };
    let chain = std::ptr::from_ref(&priority).cast::<c_void>();
    let desc = descriptor();
    // SAFETY: the adapter is wgpu-core's, and the guard is dropped before the adapter is used
    // through wgpu again.
    let hal = unsafe { adapter.as_hal::<wgpu::hal::api::Vulkan>() }
        .ok_or_else(|| "not a Vulkan adapter".to_owned())?;
    // SAFETY: the callback only adds an extension to the list and chains one structure onto
    // the queue's create info; `priority` outlives the call, which is where Vulkan reads it.
    let opened = unsafe {
        hal.open_with_callback(
            desc.required_features,
            &desc.required_limits,
            &desc.memory_hints,
            Some(Box::new(move |args| {
                if !args.extensions.contains(&EXTENSION) {
                    args.extensions.push(EXTENSION);
                }
                for info in args.queue_create_infos.iter_mut() {
                    info.p_next = chain;
                }
            })),
        )
    }
    .map_err(|e| format!("{e:?}"))?;
    drop(hal);
    // SAFETY: `opened` was just made from this adapter's own hal adapter, with the features
    // and limits `desc` names.
    let (device, queue) =
        unsafe { adapter.create_device_from_hal(opened, &desc) }.map_err(|e| e.to_string())?;
    Ok(Gpu { device, queue })
}

#[cfg(not(target_os = "linux"))]
fn open_low(_adapter: &wgpu::Adapter) -> Result<Gpu, String> {
    Err("queue priority is asked through Vulkan, on Linux".to_owned())
}

/// One fullscreen pass of the calibrated shader into a target of its own.
struct Work {
    gpu: Gpu,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    params: wgpu::Buffer,
    target: wgpu::TextureView,
}

impl Work {
    fn new(gpu: &Gpu, format: wgpu::TextureFormat, size: (u32, u32)) -> Self {
        let device = &gpu.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("load"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("load"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("load"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("load"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("load"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("load"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: params.as_entire_binding(),
            }],
        });
        let target = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("load"),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        let work = Self {
            gpu: gpu.clone(),
            pipeline,
            bind_group,
            params,
            target,
        };
        work.set_iters(1);
        work
    }

    fn set_iters(&self, iters: u32) {
        let mut bytes = [0u8; 16];
        bytes[..4].copy_from_slice(&iters.to_le_bytes());
        bytes[4..8].copy_from_slice(&0.25f32.to_le_bytes());
        self.gpu.queue.write_buffer(&self.params, 0, &bytes);
    }

    fn submit(&self) -> wgpu::SubmissionIndex {
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        self.encode(&mut encoder);
        self.gpu.queue.submit([encoder.finish()])
    }

    /// Record one pass into `encoder`.
    fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("load"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    fn wait(&self, index: wgpu::SubmissionIndex) {
        self.gpu
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(index),
                timeout: Some(WAIT),
            })
            .expect("the GPU finishes a pass");
    }

    /// The median time, in ms, from submitting `passes` passes at `iters` one by one to the
    /// last of them done, over `n` rounds back to back on an otherwise idle GPU.
    fn time(&self, iters: u32, passes: usize, n: usize) -> f64 {
        self.set_iters(iters);
        let mut samples: Vec<f64> = (0..n)
            .map(|_| {
                let t = Instant::now();
                let mut last = None;
                for _ in 0..passes {
                    last = Some(self.submit());
                }
                self.wait(last.expect("at least one pass"));
                ms(t.elapsed())
            })
            .collect();
        percentile(&mut samples, 0.5)
    }

    /// The iteration count at which `passes` passes cost closest to `target` ms, by doubling
    /// and then bisecting, and what it measures.
    fn calibrate(&self, target: f64, passes: usize) -> (u32, f64) {
        let time = |iters| self.time(iters, passes, 9);
        let (mut lo, mut hi) = (1u32, 1u32);
        let mut t_hi = time(hi);
        while t_hi < target && hi < 1 << 14 {
            lo = hi;
            hi *= 2;
            t_hi = time(hi);
        }
        let mut t_lo = time(lo);
        while hi - lo > 1 {
            let mid = lo.midpoint(hi);
            let t = time(mid);
            if t < target {
                (lo, t_lo) = (mid, t);
            } else {
                (hi, t_hi) = (mid, t);
            }
        }
        let iters = if target - t_lo < t_hi - target {
            lo
        } else {
            hi
        };
        (iters, time(iters))
    }
}

/// Other processes on this box that draw on the same GPU: a test or example binary under
/// some `target/`, or supersilvia itself. Seen every 200 ms by a thread of its own; a run
/// during which one was seen is thrown away and run again once they have gone.
mod neighbours {
    use std::sync::Once;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    static SEEN: AtomicBool = AtomicBool::new(false);
    static WATCH: Once = Once::new();

    fn present() -> Option<String> {
        let me = std::process::id().to_string();
        for entry in std::fs::read_dir("/proc").ok()?.flatten() {
            let pid = entry.file_name().to_string_lossy().into_owned();
            if pid == me || !pid.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else {
                continue;
            };
            let argv0 = String::from_utf8_lossy(cmdline.split(|&b| b == 0).next().unwrap_or(&[]))
                .into_owned();
            let drawing = (argv0.contains("/target/")
                && (argv0.contains("/deps/") || argv0.contains("/examples/")))
                || argv0.ends_with("/supersilvia")
                || argv0 == "supersilvia";
            if drawing && !argv0.contains("build-script") {
                return Some(format!("{pid} {argv0}"));
            }
        }
        None
    }

    fn watch() {
        WATCH.call_once(|| {
            std::thread::Builder::new()
                .name("neighbours".into())
                .spawn(|| {
                    loop {
                        if let Some(who) = present()
                            && !SEEN.swap(true, Ordering::Relaxed)
                        {
                            println!("(another GPU user: {who})");
                        }
                        std::thread::sleep(Duration::from_millis(200));
                    }
                })
                .expect("spawn the neighbour watch");
        });
    }

    /// Wait until nothing else has drawn for two seconds, then start counting afresh.
    pub fn quiet() {
        watch();
        let mut since = Instant::now();
        loop {
            if present().is_some() {
                since = Instant::now();
            } else if since.elapsed() >= Duration::from_secs(2) {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        SEEN.store(false, Ordering::Relaxed);
    }

    /// Whether anything else drew since the last [`quiet`].
    pub fn seen() -> bool {
        SEEN.load(Ordering::Relaxed)
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

/// The `q` quantile of `samples`, nearest rank, sorting them.
fn percentile(samples: &mut [f64], q: f64) -> f64 {
    if samples.is_empty() {
        return f64::NAN;
    }
    samples.sort_by(f64::total_cmp);
    let rank = ((samples.len() as f64 - 1.0) * q).round() as usize;
    samples[rank]
}

/// Keep the GPU busy for long enough that its clock has risen before anything is timed.
fn warm_up(work: &Work) {
    work.set_iters(200);
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(2) {
        let index = work.submit();
        work.wait(index);
    }
}

/// The synth: ticks from its own deadline, each waiting for the last one's final submission,
/// then submitting `OUTPUTS` passes one by one. Returns how many ticks it ran.
fn synth(work: &Work, stop: &AtomicBool, ticks: &AtomicU64) {
    let mut deadline = Instant::now();
    let mut last: Option<wgpu::SubmissionIndex> = None;
    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        if deadline > now {
            thread::sleep(deadline - now);
        }
        if let Some(index) = last.take() {
            work.wait(index);
        }
        for _ in 0..OUTPUTS {
            last = Some(work.submit());
        }
        ticks.fetch_add(1, Ordering::Relaxed);
        deadline += INTERVAL;
        let now = Instant::now();
        if deadline < now {
            deadline = now;
        }
    }
    if let Some(index) = last {
        work.wait(index);
    }
}

/// The same synth submitting through the renderer's own `Gpu` and `Throttle`: each pass waits
/// until at most `QUEUED_AHEAD` of the synth's earlier submissions are still on the GPU.
fn synth_throttled(
    work: &Work,
    gpu: &supersilvia::render::Gpu,
    stop: &AtomicBool,
    ticks: &AtomicU64,
) {
    let mut throttle = Throttle::new(QUEUED_AHEAD);
    let mut deadline = Instant::now();
    let mut last = None;
    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        if deadline > now {
            thread::sleep(deadline - now);
        }
        if let Some(ticket) = last.take() {
            gpu.wait(&ticket, WAIT).expect("the GPU finishes a tick");
        }
        for _ in 0..OUTPUTS {
            let mut recording = Recording::new(gpu, "load");
            work.encode(recording.encoder());
            last = throttle.submit(gpu, recording);
        }
        ticks.fetch_add(1, Ordering::Relaxed);
        deadline += INTERVAL;
        let now = Instant::now();
        if deadline < now {
            deadline = now;
        }
    }
    if let Some(ticket) = last {
        gpu.wait(&ticket, WAIT).expect("the GPU finishes a tick");
    }
}

/// A synth thread running for as long as this is held.
struct Running {
    stop: Arc<AtomicBool>,
    ticks: Arc<AtomicU64>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Running {
    fn start((work, throttled): (Arc<Work>, Option<supersilvia::render::Gpu>)) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let ticks = Arc::new(AtomicU64::new(0));
        let thread = {
            let (stop, ticks) = (stop.clone(), ticks.clone());
            thread::Builder::new()
                .name("synth".into())
                .spawn(move || match throttled {
                    Some(gpu) => synth_throttled(&work, &gpu, &stop, &ticks),
                    None => synth(&work, &stop, &ticks),
                })
                .expect("spawn the synth")
        };
        Self {
            stop,
            ticks,
            thread: Some(thread),
        }
    }

    fn ticks(&self) -> u64 {
        self.ticks.load(Ordering::Relaxed)
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().expect("the synth thread ends");
        }
    }
}

/// What one cell measured, over every run.
#[derive(Default)]
struct Cell {
    /// Every editor frame's submit-to-done time, in ms, over all runs.
    latency: Vec<f64>,
    /// Frames whose work was not done inside the interval, per run.
    overruns: Vec<usize>,
    /// Frames per run, for the rate.
    frames: Vec<usize>,
    /// The synth's ticks a second, per run.
    tick_rate: Vec<f64>,
    /// With a window: the time between eframe's frames, in ms.
    intervals: Vec<f64>,
}

impl Cell {
    fn row(&mut self, editor_ms: f64, load: &str, late_after: Option<f64>) -> String {
        let mut latency = self.latency.clone();
        let mut out = format!(
            "| {editor_ms:.1} | {load} | {:.2} | {:.2} | {:.2} | {:.2} | {} | {} |",
            percentile(&mut latency, 0.5),
            percentile(&mut latency, 0.95),
            percentile(&mut latency, 0.99),
            percentile(&mut latency, 1.0),
            join(&self.overruns, &self.frames),
            self.tick_rate
                .iter()
                .map(|r| format!("{r:.0}"))
                .collect::<Vec<_>>()
                .join(", "),
        );
        if let Some(late_after) = late_after {
            let late = self.intervals.iter().filter(|&&i| i > late_after).count();
            let mut intervals = self.intervals.clone();
            let _ = write!(
                out,
                " {:.2} | {:.2} | {:.2} | {:.2} | {late} of {} |",
                percentile(&mut intervals, 0.5),
                percentile(&mut intervals, 0.95),
                percentile(&mut intervals, 0.99),
                percentile(&mut intervals, 1.0),
                self.intervals.len(),
            );
        }
        out
    }
}

/// `overruns/frames` per run.
fn join(overruns: &[usize], frames: &[usize]) -> String {
    overruns
        .iter()
        .zip(frames)
        .map(|(o, f)| format!("{o}/{f}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Every cell: the synth off, then each load under each plan that could be opened.
fn schedule(plans: &[Plan]) -> Vec<(Option<(f64, Plan)>, f64)> {
    let mut cells = Vec::new();
    for editor_ms in EDITOR_MS {
        cells.push((None, editor_ms));
        for load in LOADS {
            for &plan in plans {
                cells.push((Some((load, plan)), editor_ms));
            }
        }
    }
    cells
}

fn load_name(load: Option<(f64, Plan)>, tick_ms: &dyn Fn(f64) -> f64) -> String {
    match load {
        None => "synth off".to_owned(),
        Some((load, plan)) => format!(
            "synth {:.0}% ({:.1} ms a tick), {}",
            load * 100.0,
            tick_ms(load),
            plan.name()
        ),
    }
}

/// Everything both modes set up: the chosen adapter, the editor's device, the synth's work
/// on each device a plan names, and the calibrated iteration counts.
struct Setup {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    editor: Gpu,
    /// The editor's device and queue as the renderer holds them, for the throttled synth.
    throttled: supersilvia::render::Gpu,
    synths: Vec<(Plan, Arc<Work>)>,
    /// Iterations of one synth pass per load, and what a tick measures at it.
    synth_iters: Vec<(f64, u32, f64)>,
    /// Iterations of the editor's pass per paint cost, and what it measures.
    editor_iters: Vec<(f64, u32, f64)>,
}

impl Setup {
    fn new(only: Option<&[Plan]>) -> Self {
        let (instance, adapter, _) =
            adapter::headless(&adapter::Asked::integrated()).unwrap_or_else(|e| panic!("{e}"));
        println!("adapter: {}", adapter::describe(&adapter.get_info()));
        let editor = open(&adapter);
        let throttled = supersilvia::render::Gpu::from_parts(
            instance.clone(),
            adapter.clone(),
            editor.device.clone(),
            editor.queue.clone(),
        );
        let one_queue = Arc::new(Work::new(
            &editor,
            wgpu::TextureFormat::Rgba16Float,
            OUTPUT_SIZE,
        ));
        let mut synths = vec![
            (Plan::OneQueue, one_queue.clone()),
            (Plan::OneQueueThrottled, one_queue),
        ];
        synths.push((
            Plan::TwoDevices,
            Arc::new(Work::new(
                &open(&adapter),
                wgpu::TextureFormat::Rgba16Float,
                OUTPUT_SIZE,
            )),
        ));
        match open_low(&adapter) {
            Ok(low) => synths.push((
                Plan::TwoDevicesLow,
                Arc::new(Work::new(
                    &low,
                    wgpu::TextureFormat::Rgba16Float,
                    OUTPUT_SIZE,
                )),
            )),
            Err(e) => println!("no low-priority device: {e}"),
        }
        if let Some(only) = only {
            synths.retain(|(plan, _)| only.contains(plan));
        }

        neighbours::quiet();
        let pass = &synths.first().expect("at least one plan runs").1;
        warm_up(pass);
        let synth_iters = LOADS
            .iter()
            .map(|&load| {
                let (iters, measured) = pass.calibrate(ms(INTERVAL) * load, OUTPUTS);
                println!(
                    "synth {:.0}%: {iters} iterations a pass, {measured:.2} ms a tick of {OUTPUTS} passes",
                    load * 100.0
                );
                (load, iters, measured)
            })
            .collect();

        let paint = Work::new(&editor, wgpu::TextureFormat::Rgba8Unorm, EDITOR_SIZE);
        let editor_iters = EDITOR_MS
            .iter()
            .map(|&target| {
                warm_up(&paint);
                let (iters, measured) = paint.calibrate(target, 1);
                println!(
                    "editor {target:.1} ms: {iters} iterations, {measured:.2} ms at {}x{}",
                    EDITOR_SIZE.0, EDITOR_SIZE.1
                );
                (target, iters, measured)
            })
            .collect();

        if neighbours::seen() {
            println!("(calibrated while another process drew; the figures above may be off)");
        }
        Self {
            instance,
            adapter,
            editor,
            throttled,
            synths,
            synth_iters,
            editor_iters,
        }
    }

    fn plans(&self) -> Vec<Plan> {
        self.synths.iter().map(|(p, _)| *p).collect()
    }

    fn synth_for(&self, load: f64, plan: Plan) -> (Arc<Work>, Option<supersilvia::render::Gpu>) {
        let work = &self
            .synths
            .iter()
            .find(|(p, _)| *p == plan)
            .expect("a plan the setup opened")
            .1;
        let iters = self
            .synth_iters
            .iter()
            .find(|(l, _, _)| *l == load)
            .expect("a calibrated load")
            .1;
        work.set_iters(iters);
        let throttled = (plan == Plan::OneQueueThrottled).then(|| self.throttled.clone());
        (work.clone(), throttled)
    }

    fn editor_iters(&self, editor_ms: f64) -> u32 {
        self.editor_iters
            .iter()
            .find(|(m, _, _)| *m == editor_ms)
            .expect("a calibrated paint")
            .1
    }

    fn tick_ms(&self, load: f64) -> f64 {
        self.synth_iters
            .iter()
            .find(|(l, _, _)| *l == load)
            .map_or(f64::NAN, |(_, _, t)| *t)
    }
}

fn header(window: bool) {
    let mut h = "| editor ms | synth | median | p95 | p99 | max | overruns/frames per 6 s run | synth ticks/s |"
        .to_owned();
    let mut rule = "| --- | --- | --- | --- | --- | --- | --- | --- |".to_owned();
    if window {
        h.push_str(" frame median | p95 | p99 | max | late |");
        rule.push_str(" --- | --- | --- | --- | --- |");
    }
    println!("{h}\n{rule}");
}

fn headless(setup: &Setup) {
    let paint = Work::new(&setup.editor, wgpu::TextureFormat::Rgba8Unorm, EDITOR_SIZE);
    println!("\nheadless: submit to done, ms, the editor at 100 Hz deadlines");
    header(false);
    for (load, editor_ms) in schedule(&setup.plans()) {
        paint.set_iters(setup.editor_iters(editor_ms));
        let mut cell = Cell::default();
        let mut runs = 0;
        while runs < RUNS {
            neighbours::quiet();
            let mut latencies = Vec::new();
            let synth = load.map(|(load, plan)| Running::start(setup.synth_for(load, plan)));
            let mut deadline = Instant::now();
            let started = deadline;
            let mut recording: Option<(Instant, u64)> = None;
            let mut overruns = 0;
            let mut frames = 0;
            loop {
                let now = Instant::now();
                if deadline > now {
                    thread::sleep(deadline - now);
                }
                if recording.is_none() && started.elapsed() >= WARM {
                    recording = Some((Instant::now(), synth.as_ref().map_or(0, Running::ticks)));
                }
                let t = Instant::now();
                let index = paint.submit();
                paint.wait(index);
                let latency = t.elapsed();
                if let Some((since, _)) = recording {
                    latencies.push(ms(latency));
                    frames += 1;
                    if latency > INTERVAL {
                        overruns += 1;
                    }
                    if since.elapsed() >= RECORD {
                        break;
                    }
                }
                deadline += INTERVAL;
                let now = Instant::now();
                while deadline < now {
                    deadline += INTERVAL;
                }
            }
            let (since, ticks0) = recording.expect("the run recorded");
            if let Some(synth) = &synth {
                cell.tick_rate
                    .push((synth.ticks() - ticks0) as f64 / since.elapsed().as_secs_f64());
            }
            drop(synth);
            if neighbours::seen() {
                cell.tick_rate.truncate(runs);
                println!("(run thrown away)");
                continue;
            }
            cell.latency.extend(latencies);
            cell.overruns.push(overruns);
            cell.frames.push(frames);
            runs += 1;
        }
        println!(
            "{}",
            cell.row(editor_ms, &load_name(load, &|l| setup.tick_ms(l)), None)
        );
    }
}

/// The window: an eframe app on wgpu at vsync, on the editor's device, submitting the editor's
/// pass from `update` and moving through the cells on a clock.
struct Window {
    setup: Arc<Setup>,
    paint: Arc<Work>,
    cells: Vec<(Option<(f64, Plan)>, f64)>,
    at: usize,
    run: usize,
    run_started: Instant,
    recording: Option<(Instant, u64)>,
    synth: Option<Running>,
    cell: Cell,
    last_frame: Option<Instant>,
    watch: mpsc::Sender<(Instant, wgpu::SubmissionIndex, bool)>,
    latencies: Arc<Mutex<Vec<f64>>>,
    overruns: usize,
    frames: usize,
    /// The display's own interval, from the first cell with the synth off.
    vsync_ms: Option<f64>,
    /// How many of the cell's intervals belong to runs kept.
    intervals_kept: usize,
}

impl Window {
    fn new(setup: Arc<Setup>) -> Self {
        let paint = Arc::new(Work::new(
            &setup.editor,
            wgpu::TextureFormat::Rgba8Unorm,
            EDITOR_SIZE,
        ));
        let latencies = Arc::new(Mutex::new(Vec::new()));
        let (watch, seen) = mpsc::channel::<(Instant, wgpu::SubmissionIndex, bool)>();
        {
            let paint = paint.clone();
            let latencies = latencies.clone();
            thread::Builder::new()
                .name("watcher".into())
                .spawn(move || {
                    for (t, index, record) in seen {
                        paint.wait(index);
                        if record {
                            latencies
                                .lock()
                                .expect("the watcher's list")
                                .push(ms(t.elapsed()));
                        }
                    }
                })
                .expect("spawn the watcher");
        }
        let cells = schedule(&setup.plans());
        println!("\nwindow: submit to done, ms, the editor at the display's vsync");
        header(true);
        let mut window = Self {
            setup,
            paint,
            cells,
            at: 0,
            run: 0,
            run_started: Instant::now(),
            recording: None,
            synth: None,
            cell: Cell::default(),
            last_frame: None,
            watch,
            latencies,
            overruns: 0,
            frames: 0,
            vsync_ms: None,
            intervals_kept: 0,
        };
        window.begin_run();
        window
    }

    fn begin_run(&mut self) {
        neighbours::quiet();
        let (load, editor_ms) = self.cells[self.at];
        self.paint.set_iters(self.setup.editor_iters(editor_ms));
        self.synth = load.map(|(load, plan)| Running::start(self.setup.synth_for(load, plan)));
        self.run_started = Instant::now();
        self.recording = None;
        self.overruns = 0;
        self.frames = 0;
    }

    /// Ends the current run; returns false when every cell is done.
    fn end_run(&mut self) -> bool {
        // Every recorded frame's wait has landed once the queue is empty.
        let done = self.paint.submit();
        self.paint.wait(done);
        thread::sleep(Duration::from_millis(50));
        let latencies = std::mem::take(&mut *self.latencies.lock().expect("the watcher's list"));
        if neighbours::seen() {
            println!("(run thrown away)");
            self.synth = None;
            self.cell.intervals.truncate(self.intervals_kept);
            self.begin_run();
            return true;
        }
        let overruns = latencies.iter().filter(|&&l| l > ms(INTERVAL)).count();
        self.cell.latency.extend(latencies);
        let (since, ticks0) = self.recording.expect("the run recorded");
        if let Some(synth) = self.synth.take() {
            self.cell
                .tick_rate
                .push((synth.ticks() - ticks0) as f64 / since.elapsed().as_secs_f64());
        }
        self.cell.overruns.push(overruns);
        self.cell.frames.push(self.frames);
        self.intervals_kept = self.cell.intervals.len();
        self.run += 1;
        if self.run == RUNS {
            let (load, editor_ms) = self.cells[self.at];
            if load.is_none() && self.vsync_ms.is_none() {
                let mut intervals = self.cell.intervals.clone();
                let vsync = percentile(&mut intervals, 0.5);
                println!("(display interval {vsync:.2} ms; a frame is late past 1.5 of it)");
                self.vsync_ms = Some(vsync);
            }
            let late_after = self.vsync_ms.map(|v| v * 1.5);
            let setup = self.setup.clone();
            println!(
                "{}",
                self.cell.row(
                    editor_ms,
                    &load_name(load, &|l| setup.tick_ms(l)),
                    late_after
                )
            );
            self.cell = Cell::default();
            self.intervals_kept = 0;
            self.run = 0;
            self.at += 1;
            if self.at == self.cells.len() {
                return false;
            }
        }
        self.begin_run();
        true
    }
}

impl eframe::App for Window {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.at == self.cells.len() {
            ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Close);
            return;
        }
        let now = Instant::now();
        let record = self.recording.is_some();
        if record && let Some(last) = self.last_frame {
            self.cell.intervals.push(ms(now - last));
        }
        self.last_frame = Some(now);
        if !record && self.run_started.elapsed() >= WARM {
            self.recording = Some((now, self.synth.as_ref().map_or(0, Running::ticks)));
        }
        let index = self.paint.submit();
        self.watch
            .send((now, index, record))
            .expect("the watcher is running");
        if record {
            self.frames += 1;
        }
        eframe::egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("queue_contention");
            let (load, editor_ms) = self.cells[self.at];
            ui.label(format!(
                "editor {editor_ms:.1} ms, {}, run {} of {RUNS}",
                load_name(load, &|l| self.setup.tick_ms(l)),
                self.run + 1
            ));
        });
        if self
            .recording
            .is_some_and(|(since, _)| since.elapsed() >= RECORD)
        {
            self.last_frame = None;
            if !self.end_run() {
                ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Close);
            }
        }
        ctx.request_repaint();
    }
}

fn main() {
    env_logger::init();
    let args: Vec<String> = std::env::args().collect();
    let window = args.iter().any(|a| a == "--window");
    let only: Option<Vec<Plan>> = args
        .iter()
        .position(|a| a == "--only")
        .and_then(|at| args.get(at + 1))
        .map(|names| {
            names
                .split(',')
                .map(|name| match name {
                    "one" => Plan::OneQueue,
                    "throttled" => Plan::OneQueueThrottled,
                    "two" => Plan::TwoDevices,
                    "low" => Plan::TwoDevicesLow,
                    other => panic!("no plan called {other}: one, throttled, two or low"),
                })
                .collect()
        });
    let setup = Arc::new(Setup::new(only.as_deref()));
    if !window {
        headless(&setup);
        return;
    }
    let existing = egui_wgpu::WgpuSetupExisting {
        instance: setup.instance.clone(),
        adapter: setup.adapter.clone(),
        device: setup.editor.device.clone(),
        queue: setup.editor.queue.clone(),
    };
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("queue_contention")
            .with_inner_size([960.0, 540.0]),
        wgpu_options: egui_wgpu::WgpuConfiguration {
            surface: egui_wgpu::SurfaceConfig {
                present_mode: wgpu::PresentMode::AutoVsync,
                desired_maximum_frame_latency: None,
            },
            wgpu_setup: egui_wgpu::WgpuSetup::Existing(existing),
            ..Default::default()
        },
        ..Default::default()
    };
    eframe::run_native(
        "queue_contention",
        options,
        Box::new(move |_cc| Ok(Box::new(Window::new(setup)))),
    )
    .expect("the window runs");
}
