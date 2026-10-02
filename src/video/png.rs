// SPDX-License-Identifier: AGPL-3.0-or-later

//! PNG in and out, through GStreamer's `pngenc` and `pngdec`.
//!
//! There is no image crate in this project and there is not going to be one: GStreamer is
//! already a dependency and already encodes and decodes PNG. Two short pipelines —
//! `appsrc ! videoconvert ! pngenc ! filesink` and `filesrc ! pngdec ! videoconvert !
//! appsink` — are the whole module.
//!
//! It lives in `video/` because that is where GStreamer lives, and `video/` takes no
//! graphical dependency: this hands out RGBA bytes and never a texture.

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use gstreamer_video::prelude::VideoFrameExt as _;
use std::path::Path;

/// Raw pixels, RGBA8, rows top first — the same convention `nodes::Frame` uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    /// A blank image, for a caller that wants to fill it in.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            rgba: vec![0; (width as usize) * (height as usize) * 4],
        }
    }
}

/// Write RGBA8 pixels to a PNG file, creating the folder above it if it is not there.
///
/// # Errors
/// The pixel slice does not match the dimensions, GStreamer is missing `pngenc`, or the
/// file could not be written.
pub fn write(path: &Path, image: &Image) -> Result<(), String> {
    let expected = (image.width as usize) * (image.height as usize) * 4;
    if image.width == 0 || image.height == 0 {
        return Err("a PNG of no size".to_string());
    }
    if image.rgba.len() != expected {
        return Err(format!(
            "{}x{} wants {expected} bytes, got {}",
            image.width,
            image.height,
            image.rgba.len()
        ));
    }
    gst::init().map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }

    let description = format!(
        "appsrc name=src ! videoconvert ! pngenc snapshot=true ! filesink location=\"{}\"",
        escape(path)
    );
    let pipeline = launch(&description)?;
    let src = pipeline
        .by_name("src")
        .ok_or("no appsrc")?
        .downcast::<gst_app::AppSrc>()
        .map_err(|_| "src is not an appsrc".to_string())?;

    let info =
        gst_video::VideoInfo::builder(gst_video::VideoFormat::Rgba, image.width, image.height)
            .fps(gst::Fraction::new(1, 1))
            .build()
            .map_err(|e| e.to_string())?;
    src.set_caps(Some(&info.to_caps().map_err(|e| e.to_string())?));
    src.set_format(gst::Format::Time);

    pipeline
        .set_state(gst::State::Playing)
        .map_err(|e| format!("{}: {e}", path.display()))?;

    // One buffer, then end of stream: `snapshot=true` makes `pngenc` finish after the first.
    let mut buffer = gst::Buffer::with_size(image.rgba.len()).map_err(|e| e.to_string())?;
    {
        let reference = buffer.get_mut().ok_or("buffer is shared")?;
        let mut map = reference.map_writable().map_err(|e| e.to_string())?;
        map.copy_from_slice(&image.rgba);
    }
    let pushed = src.push_buffer(buffer).map(|_| ());
    let ended = src.end_of_stream().map(|_| ());
    let result = pushed
        .and(ended)
        .map_err(|e| format!("{}: {e}", path.display()))
        .and_then(|()| run_to_end(&pipeline));
    let _ = pipeline.set_state(gst::State::Null);
    result
}

/// Read a PNG file back as RGBA8.
///
/// # Errors
/// The file is missing or is not a PNG this GStreamer can decode.
pub fn read(path: &Path) -> Result<Image, String> {
    if !path.is_file() {
        return Err(format!("{}: no such file", path.display()));
    }
    gst::init().map_err(|e| e.to_string())?;

    let description = format!(
        "filesrc location=\"{}\" ! pngdec ! videoconvert ! video/x-raw,format=RGBA ! \
         appsink name=sink sync=false",
        escape(path)
    );
    let pipeline = launch(&description)?;
    let sink = pipeline
        .by_name("sink")
        .ok_or("no appsink")?
        .downcast::<gst_app::AppSink>()
        .map_err(|_| "sink is not an appsink".to_string())?;

    pipeline
        .set_state(gst::State::Playing)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let sample = sink
        .pull_sample()
        .map_err(|e| format!("{}: {e}", path.display()));
    let image = sample.and_then(|sample| {
        let caps = sample.caps().ok_or("no caps".to_string())?;
        let info = gst_video::VideoInfo::from_caps(caps).map_err(|e| e.to_string())?;
        let buffer = sample.buffer().ok_or("no buffer".to_string())?;
        let frame = gst_video::VideoFrameRef::from_buffer_ref_readable(buffer, &info)
            .map_err(|e| e.to_string())?;
        Ok(rows(&frame, &info))
    });
    let _ = pipeline.set_state(gst::State::Null);
    image
}

/// Copy a decoded frame out row by row, since a plane's stride is the decoder's business
/// and is commonly wider than the picture.
fn rows(frame: &gst_video::VideoFrameRef<&gst::BufferRef>, info: &gst_video::VideoInfo) -> Image {
    let (width, height) = (info.width(), info.height());
    let stride = frame.plane_stride()[0].unsigned_abs() as usize;
    let line = width as usize * 4;
    let data = frame.plane_data(0).unwrap_or(&[]);
    let mut rgba = Vec::with_capacity(line * height as usize);
    for y in 0..height as usize {
        let start = y * stride;
        match data.get(start..start + line) {
            Some(row) => rgba.extend_from_slice(row),
            None => rgba.resize(line * height as usize, 0),
        }
    }
    Image {
        width,
        height,
        rgba,
    }
}

/// Build a pipeline from a description, naming what was missing rather than "parse error".
pub(super) fn launch(description: &str) -> Result<gst::Pipeline, String> {
    gst::parse::launch(description)
        .map_err(|e| format!("{e}"))?
        .downcast::<gst::Pipeline>()
        .map_err(|_| "not a pipeline".to_string())
}

/// Wait for end of stream or an error on the bus. Nothing here is on the frame thread: a
/// thumbnail is written by the save path and read once when a card first draws it.
pub(super) fn run_to_end(pipeline: &gst::Pipeline) -> Result<(), String> {
    let bus = pipeline.bus().ok_or("no bus")?;
    for message in bus.iter_timed(gst::ClockTime::from_seconds(10)) {
        match message.view() {
            gst::MessageView::Eos(_) => return Ok(()),
            gst::MessageView::Error(e) => return Err(format!("{}", e.error())),
            _ => {}
        }
    }
    Err("timed out".to_string())
}

/// A path inside a `gst-launch` description, where `"` and `\` are the two characters that
/// would end the string early.
pub(super) fn escape(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}
