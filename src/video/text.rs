// SPDX-License-Identifier: AGPL-3.0-or-later

//! Words into a picture, through the same GStreamer that already decodes the clips.
//!
//! `nodes/` may take no graphical dependency and a font rasterizer is one, so the letters are
//! drawn by the **pango** plugin that is in the box for subtitles: `textoverlay` composites a
//! string onto a frame of the size the node asked for, with a font description, a horizontal
//! and a vertical alignment. That is the whole of the drawing, and no font crate is taken on
//! to get it.
//!
//! **It is `textoverlay` rather than `textrender`.** The two are one plugin and one
//! rasterizer; what differs is the canvas. `textrender` sizes its output to the text it was
//! given, which would make the Size control a resolution rather than a size — a word would
//! fill the frame whatever it said. `textoverlay` draws into a frame of a stated size, which
//! is silvia's own canvas with `fillText` on it, so Size, Align and Baseline mean there what
//! they mean there.
//!
//! **One frame per string.** The pipeline carries `num-buffers=1`: it renders once, publishes
//! through the triple buffer every other source publishes through, and goes to end of stream.
//! Nothing republishes while nothing changes — the node holds the `Arc` and hands the same one
//! out every tick, which the renderer skips having seen it — and a keystroke builds a new
//! pipeline rather than a new shader.
//!
//! The picture is white on black, which is silvia's: only the coverage is used, and the two
//! colors are the patch's, mixed over it in WGSL.

use super::{Delivery, propose_video_meta, sink_chain};
use crate::nodes::Frame;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use std::sync::{Arc, Mutex};

/// Everything a rendering depends on. A tick builds one and compares it with the last, so a
/// frame is drawn when something changed and never otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    pub text: String,
    /// A font family as the machine's font list names it. A face that is not installed falls
    /// back, which is what a browser did for silvia.
    pub family: String,
    /// The pango weight keyword: `Normal`, `Bold` or `Light`.
    pub weight: &'static str,
    /// Pixels, not points — the size is a size on this canvas, as silvia's `64px` is.
    pub size: u32,
    /// `left`, `center` or `right`.
    pub align: &'static str,
    /// `top`, `center` or `bottom`, which is `textoverlay`'s spelling of silvia's baseline.
    pub baseline: &'static str,
    pub width: u32,
    pub height: u32,
}

impl Spec {
    /// The pango font description the size, the family and the weight come to.
    ///
    /// The comma ends the family. Without it pango reads a family's last word as a style
    /// where it is one — `Times New Roman` as `Times New` in roman, `DIN Condensed` as `DIN`
    /// condensed — and draws the fallback.
    fn font_desc(&self) -> String {
        format!("{}, {} {}px", self.family, self.weight, self.size)
    }
}

/// One rendered string, and the pipeline that drew it.
pub struct Words {
    pipeline: gst::Pipeline,
    output: triple_buffer::Output<Option<Arc<Frame>>>,
    error: Arc<Mutex<Option<String>>>,
}

impl Words {
    /// Draw a string. Returns as soon as the pipeline is playing; the frame arrives on
    /// GStreamer's own thread and is taken by [`Words::latest`].
    pub fn render(spec: &Spec) -> Result<Self, String> {
        gst::init().map_err(|e| format!("gstreamer: {e}"))?;

        // A black frame of the asked-for size, the string composited onto it, and the sink
        // chain every other source here ends in. The string itself is set as a property
        // rather than spliced into this, because a newline and a quotation mark are both
        // things a person types and neither survives `parse::launch`.
        let description = format!(
            "videotestsrc pattern=black num-buffers=1 \
             ! video/x-raw,format=RGBA,width={},height={},framerate=1/1 \
             ! textoverlay name=words ! {}",
            spec.width,
            spec.height,
            sink_chain(Delivery::Rgba).ok_or("no sink chain")?,
        );
        let pipeline = gst::parse::launch(&description)
            .map_err(|e| format!("{description}: {e}"))?
            .downcast::<gst::Pipeline>()
            .map_err(|_| "not a pipeline".to_string())?;

        let words = pipeline.by_name("words").ok_or("no textoverlay")?;
        words.set_property("text", &spec.text);
        words.set_property("font-desc", spec.font_desc());
        // The size is in the font description and must stay there: `auto-resize` would scale
        // it by the frame's own height against a 640x480 reference, so the same Size would
        // mean a different picture at each Texture Size.
        words.set_property("auto-resize", false);
        // silvia draws the glyphs and nothing else. An outline and a shadow are both ink in
        // the mask, so both would end up mixed as if they were letters.
        words.set_property("draw-outline", false);
        words.set_property("draw-shadow", false);
        words.set_property("shaded-background", false);
        // White: only the coverage is read, and the colors are the patch's.
        words.set_property("color", 0xffff_ffffu32);
        words.set_property_from_str("halignment", spec.align);
        // silvia's one Align governs both where the block sits and how its lines sit against
        // each other, because a canvas `textAlign` does both.
        words.set_property_from_str("line-alignment", spec.align);
        words.set_property_from_str("valignment", spec.baseline);

        let sink = pipeline
            .by_name("sink")
            .ok_or("no appsink")?
            .downcast::<gst_app::AppSink>()
            .map_err(|_| "sink is not an appsink".to_string())?;
        let (input, output) = triple_buffer::TripleBuffer::new(&None).split();
        let input = Mutex::new(input);
        let error = Arc::new(Mutex::new(None));
        let sink_error = Arc::clone(&error);
        sink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .propose_allocation(propose_video_meta)
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    match super::clip::frame_from(&sample, Delivery::Rgba) {
                        Ok(frame) => {
                            if let Ok(mut input) = input.lock() {
                                input.write(Some(Arc::new(frame)));
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

        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| format!("{description}: {e}"))?;
        Ok(Self {
            pipeline,
            output,
            error,
        })
    }

    /// The frame, once it has been drawn. `None` until then, and never waits.
    pub fn latest(&mut self) -> Option<Arc<Frame>> {
        self.output.read().clone()
    }

    /// A failure from the sink or the bus.
    ///
    /// End of stream is not one here, unlike a camera's: one buffer is all this pipeline was
    /// ever going to produce, so reaching the end of it is the rendering having finished.
    pub fn error(&self) -> Option<String> {
        if let Some(bus) = self.pipeline.bus()
            && let Some(msg) = bus.pop_filtered(&[gst::MessageType::Error])
            && let gst::MessageView::Error(e) = msg.view()
            && let Ok(mut slot) = self.error.lock()
        {
            *slot = Some(format!("text: {}", e.error()));
        }
        self.error.lock().ok().and_then(|s| s.clone())
    }
}

impl Drop for Words {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

/// The families this machine can draw, as a Font menu: pango's three generic names first,
/// then every installed family by its first name, in order.
///
/// Asked of the machine — fontconfig on Linux, AppKit's font collection on macOS — once and
/// kept, leaked, for the life of the process: the menu is `'static` like every other
/// option's, and a font installed while the app runs appears on the next launch. A face's other names (`Cantarell Light`,
/// `CaskaydiaMono NFM`) are left off: the Weight row asks for the light or bold of a family,
/// and pango resolves the rest.
///
/// Empty where the machine cannot be asked, and the menu then falls back to the node's
/// declared choices.
pub fn families() -> &'static [(&'static str, &'static str)] {
    static FAMILIES: std::sync::OnceLock<Vec<(&'static str, &'static str)>> =
        std::sync::OnceLock::new();
    FAMILIES.get_or_init(|| {
        let Some(installed) = crate::platform::fonts::installed() else {
            return Vec::new();
        };
        ["Sans", "Serif", "Monospace"]
            .into_iter()
            .chain(
                installed
                    .into_iter()
                    .map(|f| &*Box::leak(f.into_boxed_str())),
            )
            .map(|f| (f, f))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The menu leads with the generic names, which draw on any machine, and lists each
    /// installed family once.
    #[test]
    fn the_font_menu_is_the_machines_families() {
        let menu = families();
        assert!(
            menu.len() > 3,
            "this machine lists no families of its own: {menu:?}"
        );
        assert_eq!(
            &menu[..3],
            &[
                ("Sans", "Sans"),
                ("Serif", "Serif"),
                ("Monospace", "Monospace")
            ]
        );
        let mut seen = std::collections::HashSet::new();
        assert!(
            menu[3..].iter().all(|(f, _)| seen.insert(*f)),
            "a family listed twice"
        );
    }

    fn spec(text: &str) -> Spec {
        Spec {
            text: text.to_string(),
            family: "Sans".to_string(),
            weight: "Bold",
            size: 64,
            align: "center",
            baseline: "center",
            width: 320,
            height: 180,
        }
    }

    /// A string is drawn as white ink on black, in a frame of the size that was asked for.
    #[test]
    fn a_string_is_drawn_as_ink_on_black() {
        let mut words = Words::render(&spec("HI")).expect("a pipeline");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let frame = loop {
            if let Some(f) = words.latest() {
                break f;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "nothing was drawn: {:?}",
                words.error()
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!((frame.width, frame.height), (320, 180));
        let bytes = frame.bytes().expect("bytes were asked for");
        let ink = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[0] > 200)
            .count();
        assert!(ink > 100, "two capitals are more ink than {ink} pixels");
        assert!(
            ink < 320 * 180 / 2,
            "and less than half the frame: {ink} pixels"
        );
        assert_eq!(words.error(), None);
    }

    /// A family on the menu is the face that draws: a monospace family sets the words
    /// differently from a name no machine has, which pango answers with its fallback.
    #[test]
    fn a_family_on_the_menu_draws_as_itself() {
        const MONOSPACE: [&str; 3] = ["Menlo", "DejaVu Sans Mono", "Liberation Mono"];
        let Some(&(family, _)) = families().iter().find(|(f, _)| MONOSPACE.contains(f)) else {
            eprintln!("none of {MONOSPACE:?} is on this machine's menu; skipping");
            return;
        };
        let draw = |family: &str| {
            let spec = Spec {
                family: family.to_string(),
                weight: "Normal",
                ..spec("Wil Wil")
            };
            drawn(&mut Words::render(&spec).expect("a pipeline"))
                .bytes()
                .expect("bytes were asked for")
                .to_vec()
        };
        let listed = draw(family);
        let fallback = draw("No Such Family Anywhere");
        assert_ne!(listed, fallback, "{family} drew as the fallback does");
    }

    /// The frame a rendering produces, within ten seconds.
    fn drawn(words: &mut Words) -> Arc<Frame> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if let Some(f) = words.latest() {
                return f;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "nothing was drawn: {:?}",
                words.error()
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// An empty string is a black frame rather than a failure.
    #[test]
    fn an_empty_string_draws_nothing_and_fails_at_nothing() {
        let mut words = Words::render(&spec("")).expect("a pipeline");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let frame = loop {
            if let Some(f) = words.latest() {
                break f;
            }
            assert!(std::time::Instant::now() < deadline, "nothing was drawn");
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        let bytes = frame.bytes().expect("bytes were asked for");
        assert!(
            bytes.as_chunks::<4>().0.iter().all(|p| p[0] < 16),
            "no ink anywhere"
        );
    }

    /// The size is in pixels of this frame, which is what keeps Size meaning a size, and the
    /// family is closed by a comma, which keeps a last word like `Roman` in its name.
    #[test]
    fn the_font_description_asks_for_pixels() {
        assert_eq!(spec("x").font_desc(), "Sans, Bold 64px");
    }
}
