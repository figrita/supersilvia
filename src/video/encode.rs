// SPDX-License-Identifier: AGPL-3.0-or-later

//! A video file written one frame at a time, for an offline render.
//!
//! The same hardware encoder `clip.rs` probes for the import transcode, in a delivery's
//! shape rather than the cache's: constant quality and the encoder's own keyframe interval,
//! since a render is a thing you send someone and nobody scrubs it here. Frames go in as
//! RGBA8 rows-top-first — what `nodes::Frame` and the capture readback already are — each
//! stamped with its index over the frame rate, so the file's clock is the render's and not
//! the wall's. It writes to a `.part` beside the destination and renames at the end, so a
//! half-written file is never mistaken for a clip.
//!
//! **Alpha is dropped** by the conversion to the encoder's format, so the render hands a
//! frame over premultiplied, as the graph holds it (`render::readback::Alpha`): premultiplied
//! color with its alpha dropped is the picture over black, which is what every viewer shows.

use crate::video::clip::Codec;
use crate::video::png::{escape, launch, run_to_end};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use std::path::{Path, PathBuf};

/// An encode in progress.
pub struct Encoder {
    pipeline: gst::Pipeline,
    src: gst_app::AppSrc,
    dest: PathBuf,
    partial: PathBuf,
    width: u32,
    height: u32,
    /// Nanoseconds per frame.
    frame_ns: u64,
    /// Frames pushed so far, which is also the next frame's index.
    frames: u64,
}

impl Encoder {
    /// Open `dest` for `width`x`height` frames at `fps`, on the machine's hardware encoder.
    ///
    /// # Errors
    /// No hardware encoder, a size of nothing, or a folder that could not be made.
    pub fn start(dest: &Path, width: u32, height: u32, fps: f64) -> Result<Self, String> {
        if width == 0 || height == 0 {
            return Err("a video of no size".to_string());
        }
        if fps.is_nan() || fps <= 0.0 {
            return Err(format!("a video at {fps} fps"));
        }
        let codec = Codec::probe().ok_or_else(|| {
            format!(
                "no hardware video encoder: {} is needed to write a video",
                crate::platform::video::CODEC_HINT
            )
        })?;
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let partial = dest.with_extension("part");
        // The rate as a fraction with a thousandth's precision, so 29.97 survives.
        let rate = gst::Fraction::new((fps * 1000.0).round() as i32, 1000);
        let description = format!(
            "appsrc name=src format=time is-live=false ! videoconvert ! queue ! {} ! mp4mux \
             ! filesink location=\"{}\"",
            codec.delivery_chain(),
            escape(&partial),
        );
        let pipeline = launch(&description)?;
        crate::platform::video::settle_before_eos(&pipeline);
        let src = pipeline
            .by_name("src")
            .ok_or("no appsrc")?
            .downcast::<gst_app::AppSrc>()
            .map_err(|_| "src is not an appsrc".to_string())?;
        let info = gst_video::VideoInfo::builder(gst_video::VideoFormat::Rgba, width, height)
            .fps(rate)
            .build()
            .map_err(|e| e.to_string())?;
        src.set_caps(Some(&info.to_caps().map_err(|e| e.to_string())?));
        src.set_format(gst::Format::Time);
        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| format!("{}: {e}", dest.display()))?;
        Ok(Self {
            pipeline,
            src,
            dest: dest.to_path_buf(),
            partial,
            width,
            height,
            frame_ns: (1.0e9 / fps).round() as u64,
            frames: 0,
        })
    }

    /// Frames pushed so far.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// One frame, RGBA8 rows top first, at the size the encoder was opened with.
    ///
    /// # Errors
    /// The wrong number of bytes, or the pipeline refused it.
    pub fn push(&mut self, rgba: &[u8]) -> Result<(), String> {
        let expected = (self.width as usize) * (self.height as usize) * 4;
        if rgba.len() != expected {
            return Err(format!(
                "{}x{} wants {expected} bytes, got {}",
                self.width,
                self.height,
                rgba.len()
            ));
        }
        let mut buffer = gst::Buffer::with_size(rgba.len()).map_err(|e| e.to_string())?;
        {
            let reference = buffer.get_mut().ok_or("buffer is shared")?;
            reference.set_pts(gst::ClockTime::from_nseconds(self.frames * self.frame_ns));
            reference.set_duration(gst::ClockTime::from_nseconds(self.frame_ns));
            let mut map = reference.map_writable().map_err(|e| e.to_string())?;
            map.copy_from_slice(rgba);
        }
        self.src
            .push_buffer(buffer)
            .map_err(|e| format!("{}: {e}", self.dest.display()))?;
        self.frames += 1;
        Ok(())
    }

    /// End the stream, wait for the file to close, and put it under its name.
    ///
    /// # Errors
    /// The encoder failed on the way out, or the rename did.
    pub fn finish(self) -> Result<PathBuf, String> {
        let ended = self
            .src
            .end_of_stream()
            .map(|_| ())
            .map_err(|e| format!("{}: {e}", self.dest.display()));
        let result = ended.and_then(|()| run_to_end(&self.pipeline));
        let _ = self.pipeline.set_state(gst::State::Null);
        match result {
            Ok(()) => {
                std::fs::rename(&self.partial, &self.dest)
                    .map_err(|e| format!("{}: {e}", self.dest.display()))?;
                Ok(self.dest.clone())
            }
            Err(e) => {
                let _ = std::fs::remove_file(&self.partial);
                Err(e)
            }
        }
    }
}

impl Drop for Encoder {
    /// A dropped encoder is an abandoned file: stop the pipeline and take the partial away.
    /// After `finish` the partial is already renamed, so there is nothing to take.
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
        let _ = std::fs::remove_file(&self.partial);
    }
}
