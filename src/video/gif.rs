// SPDX-License-Identifier: AGPL-3.0-or-later

//! An animated GIF written one frame at a time, for an offline render.
//!
//! Through the `image` crate's encoder, already in the binary for `imagegif`'s decoding:
//! GStreamer has a GIF decoder on no machine this has run on, and an encoder on fewer. Each
//! frame is quantized to its own 256 colors (NeuQuant) and the file repeats forever, which
//! is what a loop is posted as. It writes to a `.part` beside the destination and renames at
//! the end, so a half-written file is never mistaken for a picture, as the video writer does.
//!
//! **Frames go in straight**, as a PNG's do (`render::readback::Alpha`). A GIF's alpha is one
//! bit, and a pixel is cut at half coverage before it is encoded: below half it is
//! transparent, from half up opaque at its own color. The encoder would otherwise make every
//! pixel above zero opaque, and an antialiased edge a texel wider than the shape.
//!
//! **A GIF counts time in hundredths of a second.** Frame `i` is held from `round(100 i ÷
//! fps)` to `round(100 (i + 1) ÷ fps)` hundredths, so a 30 fps loop alternates 3 and 4 and
//! the whole loop lasts what it should to the hundredth, where one rounded delay for every
//! frame would drift. Browsers hold a frame of under two hundredths for ten, so a GIF plays
//! as rendered at 50 fps and under.

use image::codecs::gif::{GifEncoder, Repeat};
use image::{Delay, Frame, RgbaImage};
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

/// NeuQuant's sampling: 1 is the best palette and the slowest, 30 the fastest. The `gif`
/// crate's own default.
const SPEED: i32 = 10;

/// A GIF in progress.
pub struct Writer {
    encoder: GifEncoder<BufWriter<File>>,
    dest: PathBuf,
    partial: PathBuf,
    width: u32,
    height: u32,
    fps: f64,
    /// Frames pushed so far, which is also the next frame's index.
    frames: u64,
}

impl Writer {
    /// Open `dest` for `width`x`height` frames at `fps`.
    ///
    /// # Errors
    /// A size of nothing, a rate of nothing, or a file that could not be made.
    pub fn start(dest: &Path, width: u32, height: u32, fps: f64) -> Result<Self, String> {
        if width == 0 || height == 0 || width > u32::from(u16::MAX) || height > u32::from(u16::MAX)
        {
            return Err(format!("a GIF cannot be {width}x{height}"));
        }
        if fps.is_nan() || fps <= 0.0 {
            return Err(format!("a GIF at {fps} fps"));
        }
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let partial = dest.with_extension("part");
        let file = File::create(&partial).map_err(|e| format!("{}: {e}", partial.display()))?;
        let mut encoder = GifEncoder::new_with_speed(BufWriter::new(file), SPEED);
        encoder
            .set_repeat(Repeat::Infinite)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            encoder,
            dest: dest.to_path_buf(),
            partial,
            width,
            height,
            fps,
            frames: 0,
        })
    }

    /// Add one frame of RGBA8, rows top first.
    ///
    /// # Errors
    /// A frame of the wrong size, or a write that failed.
    pub fn push(&mut self, rgba: &[u8]) -> Result<(), String> {
        let mut pixels = rgba.to_vec();
        for px in pixels.as_chunks_mut::<4>().0 {
            cut_at_half(px);
        }
        let image = RgbaImage::from_raw(self.width, self.height, pixels)
            .ok_or_else(|| "a frame of the wrong size".to_string())?;
        let hundredths = delay(self.frames, self.fps);
        let frame = Frame::from_parts(image, 0, 0, Delay::from_numer_denom_ms(hundredths * 10, 1));
        self.encoder
            .encode_frame(frame)
            .map_err(|e| format!("{}: {e}", self.partial.display()))?;
        self.frames += 1;
        Ok(())
    }

    /// Close the file and put it where it was asked for.
    ///
    /// # Errors
    /// The rename failed.
    pub fn finish(self) -> Result<PathBuf, String> {
        let Self {
            encoder,
            dest,
            partial,
            ..
        } = self;
        // The encoder writes the trailer as it drops, and the buffer flushes behind it.
        drop(encoder);
        std::fs::rename(&partial, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
        Ok(dest)
    }
}

/// How many hundredths of a second frame `i` is held at `fps`: the whole loop to the
/// hundredth, rather than one rounded delay repeated.
fn delay(i: u64, fps: f64) -> u32 {
    let at = |n: u64| (n as f64 * 100.0 / fps).round() as u64;
    u32::try_from(at(i + 1).saturating_sub(at(i))).unwrap_or(u32::MAX)
}

/// One straight RGBA pixel made one-bit: transparent black below half coverage, opaque at its
/// own color from half up.
fn cut_at_half(px: &mut [u8; 4]) {
    if px[3] < 128 {
        *px = [0; 4];
    } else {
        px[3] = 255;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Coverage is cut at half: a faint edge is gone rather than a texel of solid color, and
    /// a mostly covered one keeps its color, opaque.
    #[test]
    fn a_pixel_is_cut_at_half_coverage() {
        let mut faint = [0, 255, 255, 64];
        cut_at_half(&mut faint);
        assert_eq!(faint, [0; 4]);
        let mut half = [0, 255, 255, 128];
        cut_at_half(&mut half);
        assert_eq!(half, [0, 255, 255, 255]);
    }

    /// At 30 fps the delays alternate so thirty frames last one second; at 25 fps they are
    /// all four.
    #[test]
    fn the_delays_add_up_to_the_loop() {
        let total: u32 = (0..30).map(|i| delay(i, 30.0)).sum();
        assert_eq!(total, 100);
        assert!((0..30).all(|i| (3..=4).contains(&delay(i, 30.0))));
        assert!((0..25).all(|i| delay(i, 25.0) == 4));
    }

    /// A written GIF decodes to the frames it was given, repeating.
    #[test]
    fn a_gif_round_trips() {
        use image::AnimationDecoder as _;
        let dir = std::env::temp_dir().join(format!("supersilvia-gif-{}", std::process::id()));
        let path = dir.join("loop.gif");
        let mut w = Writer::start(&path, 4, 2, 10.0).unwrap();
        for shade in [0u8, 128, 255] {
            w.push(&[shade; 4 * 4 * 2]).unwrap();
        }
        assert_eq!(w.finish().unwrap(), path);
        let file = std::io::BufReader::new(File::open(&path).unwrap());
        let frames = image::codecs::gif::GifDecoder::new(file)
            .unwrap()
            .into_frames()
            .collect_frames()
            .unwrap();
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[2].buffer().get_pixel(0, 0).0[0], 255);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
