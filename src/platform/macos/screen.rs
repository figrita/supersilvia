// SPDX-License-Identifier: AGPL-3.0-or-later

//! Screen capture on macOS: ScreenCaptureKit's own picker, and a stream that writes its frames
//! into the slots a [`crate::video::Camera`] reads, with no pipeline between.
//!
//! **The picker is `SCContentSharingPicker`**, macOS's counterpart of the portal's: the system
//! draws it, and the choice it answers with is a content filter. [`ask`] sets it up on the main
//! queue and returns at once; the answer comes to an observer on whichever thread the system
//! calls it on, and goes back through the channel [`Pending::poll`] reads, as Linux's does. A
//! filter cannot cross threads, so the stream is built where the answer arrives, and what
//! crosses is a [`Cast`]. Each answer, a choice or a cancel, is the oldest ask's.
//!
//! **The picker allows as many streams as are running and asked for.** Its
//! `maximumStreamCount` is one unless set (Apple's documentation: "The default value is 1"),
//! and while that many streams it started are running, `present` shows nothing until one
//! stops — so the Main Input capturing a window left a Screen Capture node waiting on a picker
//! that never came. Before each `present` the count is set to the streams running plus the
//! asks waiting, as Electron's own `SCContentSharingPicker` patch sets it, and it falls again as
//! each stream stops. The picker stays active while either is above zero, not only while an ask
//! waits: `isActive` is the picker "available for managing capture", and the streams it started
//! are what it manages.
//!
//! **Frames skip GStreamer.** The stream delivers `32BGRA` at the content's own pixel size, the
//! pointer drawn in as Linux embeds it, on a serial queue of its own; each sample's pixel buffer
//! is published as its `IOSurface`, which the renderer samples in place, beside the same
//! buffer locked as bytes ([`super::pixels`]), and a sample with none — an idle
//! frame, the screen unchanged — is skipped.
//!
//! **The end reaches the error slot two ways**, and [`crate::video::Camera::error`] reads it
//! there as it reads a pipeline's end of stream. The stream's delegate hears
//! `stream:didStopWithError:`, which the menu bar's *Stop sharing* causes with
//! `SCStreamErrorUserStopped`; its error is declared non-null but has been seen to arrive nil
//! (Apple's forums, macOS 14.7), so it is taken as optional and a nil one is a plain stop. And a
//! sample whose `SCStreamFrameInfoStatus` is `SCFrameStatusStopped` — "the stream was stopped",
//! in `SCStream.h` — writes the same line, for a stop the delegate is not told of.
//! `proposals/macos-media.md`, section 4, has the shape that lost: an `appsrc` bridge.
//!
//! **It always asks**, as Linux does: nothing about the choice is kept.
//!
//! **The `unsafe` here** is ScreenCaptureKit's calls, which its bindings mark `unsafe` almost
//! throughout, the two Objective-C classes `define_class!` declares, and `Send` on the session a
//! [`Cast`] holds. Each block says why it holds. The app needs no Screen Recording grant for
//! what the person chose in the system's own picker (**unverified**; macOS 15 did not say).

use super::pixels;
use crate::nodes::Frame;
use crate::platform::screen::{Head, Slot};
use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send};
use objc2_core_foundation::{CFDictionary, CFNumber, CFString, CFType};
use objc2_core_media::CMSampleBuffer;
use objc2_core_video::kCVPixelFormatType_32BGRA;
use objc2_foundation::{NSError, NSNumber};
use objc2_screen_capture_kit::{
    SCContentFilter, SCContentSharingPicker, SCContentSharingPickerObserver, SCFrameStatus,
    SCStream, SCStreamConfiguration, SCStreamDelegate, SCStreamFrameInfoStatus, SCStreamOutput,
    SCStreamOutputType,
};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};

/// `SCStreamErrorUserStopped`, from `SCError.h`: the person stopped sharing from the menu bar.
const USER_STOPPED: isize = -3817;

/// Where an ask's answer goes.
type Answer = mpsc::Sender<Result<Cast, String>>;

/// The asks the picker has not answered, oldest first.
static WAITING: Mutex<VecDeque<Answer>> = Mutex::new(VecDeque::new());

/// The streams started from the picker's answers and not yet stopped by their [`Cast`]. A
/// stream the person stopped counts until then, which only ever allows the picker one more.
static STREAMS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// The picker's observer, made on the main thread by the first ask and kept there.
    static OBSERVER: RefCell<Option<Retained<Observer>>> = const { RefCell::new(None) };
}

/// A running screen capture. Dropping it stops the stream.
pub struct Cast {
    frame: Slot<Arc<Frame>>,
    error: Slot<String>,
    description: String,
    session: Option<Session>,
}

impl Cast {
    /// What a [`crate::video::Camera`] reads this cast by, valid for as long as this value is
    /// alive.
    pub fn stream(&self) -> Stream {
        Stream {
            frame: Arc::clone(&self.frame),
            error: Arc::clone(&self.error),
            description: self.description.clone(),
        }
    }
}

impl Drop for Cast {
    fn drop(&mut self) {
        if let Some(session) = self.session.take() {
            DispatchQueue::main().exec_async(move || session.stop());
        }
    }
}

/// The stream, and the object it hands its samples and its end to.
struct Session {
    stream: Retained<SCStream>,
    capture: Retained<Capture>,
    queue: DispatchRetained<DispatchQueue>,
}

// SAFETY: Objective-C reference counting is atomic, so the three may be released on any thread;
// the one call made through them after they cross, `stop`, runs on the main queue, and the
// capture's own state is behind mutexes.
unsafe impl Send for Session {}

impl Session {
    /// Stop the stream and take its output off, then let go of both once the sample queue has
    /// run whatever it was already running.
    fn stop(self) {
        // SAFETY: a stream this session started; stopping one already stopped by the person is
        // refused with an error that nothing waits for.
        unsafe { self.stream.stopCaptureWithCompletionHandler(None) };
        // SAFETY: the output this session added, of the type it was added as.
        let removed = unsafe {
            self.stream.removeStreamOutput_type_error(
                ProtocolObject::from_ref(&*self.capture),
                SCStreamOutputType::Screen,
            )
        };
        if let Err(e) = removed {
            log::debug!("screen capture: {}", e.localizedDescription());
        }
        STREAMS.fetch_sub(1, Ordering::Relaxed);
        settle(false);
        let queue = self.queue.clone();
        queue.exec_async(move || drop(self));
    }
}

/// A cast as a [`crate::video::Camera`] names it: the slots its stream writes. The [`Cast`] it
/// came from must outlive the camera — dropping it stops the stream.
#[derive(Clone)]
pub struct Stream {
    frame: Slot<Arc<Frame>>,
    error: Slot<String>,
    description: String,
}

impl Stream {
    /// The slots this stream's frames and its end are written into.
    pub fn head(&self) -> Head {
        Head::Slots {
            frame: Arc::clone(&self.frame),
            error: Arc::clone(&self.error),
            description: self.description.clone(),
        }
    }
}

impl PartialEq for Stream {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.frame, &other.frame)
    }
}

impl Eq for Stream {}

impl std::fmt::Debug for Stream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Stream").field(&self.description).finish()
    }
}

/// An ask in flight: the picker may be on screen, or the person may be ignoring it.
pub struct Pending(mpsc::Receiver<Result<Cast, String>>);

impl Pending {
    /// The answer, if there is one yet. Never waits.
    pub fn poll(&self) -> Option<Result<Cast, String>> {
        match self.0.try_recv() {
            Ok(answer) => Some(answer),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("screen capture was canceled".to_string()))
            }
        }
    }
}

/// Ask for a screen or a window through the system's picker. Returns at once.
pub fn ask() -> Pending {
    let (answer, pending) = mpsc::channel();
    DispatchQueue::main().exec_async(move || {
        WAITING
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(answer);
        // SAFETY: the shared picker is a process-wide singleton, asked for on the main queue.
        let picker = unsafe { SCContentSharingPicker::sharedPicker() };
        OBSERVER.with(|held| {
            let mut held = held.borrow_mut();
            if held.is_none() {
                let observer = Observer::new();
                // SAFETY: the observer is kept in `OBSERVER` for the life of the main thread,
                // so the picker never calls one that has gone.
                unsafe { picker.addObserver(ProtocolObject::from_ref(&*observer)) };
                *held = Some(observer);
            }
        });
        settle(true);
    });
    Pending(pending)
}

/// Answer the oldest ask still waiting with what `result` makes, if any ask is, then set the
/// picker for what is left: up again for the next ask, and active for as long as an ask waits
/// or a stream it started runs.
fn answer(result: impl FnOnce() -> Result<Cast, String>) {
    reply(
        || {
            WAITING
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()
        },
        result,
    );
    DispatchQueue::main().exec_async(|| settle(true));
}

/// Send what `result` makes to the first ask `next` hands out whose `Pending` is still there,
/// passing over the ones that have gone. `result` is not called when no ask waits, and what
/// nobody takes is dropped here, which stops a cast.
fn reply<T>(mut next: impl FnMut() -> Option<mpsc::Sender<T>>, result: impl FnOnce() -> T) {
    let Some(first) = next() else {
        return;
    };
    let mut value = result();
    let mut to = Some(first);
    while let Some(ask) = to {
        match ask.send(value) {
            Ok(()) => return,
            Err(mpsc::SendError(back)) => {
                value = back;
                to = next();
            }
        }
    }
}

/// What the shared picker is set to.
#[derive(Debug, PartialEq, Eq)]
struct Setting {
    /// `isActive`: the picker appears, and manages the streams it started.
    active: bool,
    /// `maximumStreamCount`: how many streams it allows at once.
    maximum: usize,
}

/// The picker's setting for `waiting` asks and `streams` of its own running: one stream for
/// each of both, and active while there is any.
fn setting(waiting: usize, streams: usize) -> Setting {
    let maximum = waiting + streams;
    Setting {
        active: maximum > 0,
        maximum,
    }
}

/// Set the shared picker for the asks waiting and the streams running, and put it up when
/// `present` says to and an ask is waiting. On the main queue.
fn settle(present: bool) {
    let waiting = WAITING.lock().unwrap_or_else(PoisonError::into_inner).len();
    let wanted = setting(waiting, STREAMS.load(Ordering::Relaxed));
    let maximum = NSNumber::new_usize(wanted.maximum);
    // SAFETY: the shared picker, on the main queue, with the observer the first ask added
    // still listening; each is a plain property, set before the `present` that reads them.
    unsafe {
        let picker = SCContentSharingPicker::sharedPicker();
        picker.setMaximumStreamCount(Some(&maximum));
        picker.setActive(wanted.active);
        if present && waiting > 0 {
            picker.present();
        }
    }
}

/// A stream over what the person chose, started: `32BGRA` at the content's own pixel size,
/// the pointer drawn in.
fn start(filter: &SCContentFilter) -> Result<Cast, String> {
    let frame: Slot<Arc<Frame>> = Arc::default();
    let error: Slot<String> = Arc::default();
    // SAFETY: a filter the picker handed over, read for its size.
    let (rect, scale) = unsafe { (filter.contentRect(), filter.pointPixelScale()) };
    let pixels = |points: f64| (points * f64::from(scale)).round().max(1.0) as usize;
    let (width, height) = (pixels(rect.size.width), pixels(rect.size.height));
    // SAFETY: a new configuration, set before any stream reads it.
    let config = unsafe { SCStreamConfiguration::new() };
    // SAFETY: as above; each is a plain property.
    unsafe {
        config.setWidth(width);
        config.setHeight(height);
        config.setPixelFormat(kCVPixelFormatType_32BGRA);
        config.setShowsCursor(true);
    }
    let capture = Capture::new(Arc::clone(&frame), Arc::clone(&error));
    // SAFETY: the filter and configuration are valid, and the delegate is kept alive by the
    // session for as long as the stream.
    let stream = unsafe {
        SCStream::initWithFilter_configuration_delegate(
            SCStream::alloc(),
            filter,
            &config,
            Some(ProtocolObject::from_ref(&*capture)),
        )
    };
    let queue = DispatchQueue::new("supersilvia.screen", None);
    // SAFETY: the output is kept alive by the session, and the queue is serial, so the
    // output's method never runs twice at once.
    unsafe {
        stream.addStreamOutput_type_sampleHandlerQueue_error(
            ProtocolObject::from_ref(&*capture),
            SCStreamOutputType::Screen,
            Some(&queue),
        )
    }
    .map_err(|e| format!("screen capture: {}", e.localizedDescription()))?;
    // SAFETY: a stream with its output added; a start that fails ends the stream, which the
    // delegate hears.
    unsafe { stream.startCaptureWithCompletionHandler(None) };
    STREAMS.fetch_add(1, Ordering::Relaxed);
    Ok(Cast {
        frame,
        error,
        description: format!("ScreenCaptureKit {width}x{height} BGRA"),
        session: Some(Session {
            stream,
            capture,
            queue,
        }),
    })
}

/// Put `value` in `slot`, and let go of what it held outside the lock.
fn put<T>(slot: &Slot<T>, value: T) {
    let displaced = slot
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .replace(value);
    drop(displaced);
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and this adds no `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "SupersilviaScreenPicker"]
    /// The picker's observer: each answer goes to the oldest ask.
    struct Observer;

    // SAFETY: `NSObjectProtocol` has no requirements beyond being an object.
    unsafe impl NSObjectProtocol for Observer {}

    // SAFETY: each method has the signature the protocol declares.
    unsafe impl SCContentSharingPickerObserver for Observer {
        #[unsafe(method(contentSharingPicker:didCancelForStream:))]
        fn did_cancel(&self, _picker: &SCContentSharingPicker, stream: Option<&SCStream>) {
            // A stream is named only when the picker changed a running one, which nothing here
            // asked for.
            if stream.is_none() {
                answer(|| Err("no screen was chosen".to_string()));
            }
        }

        #[unsafe(method(contentSharingPicker:didUpdateWithFilter:forStream:))]
        fn did_update(
            &self,
            _picker: &SCContentSharingPicker,
            filter: &SCContentFilter,
            stream: Option<&SCStream>,
        ) {
            if stream.is_none() {
                answer(|| start(filter));
            }
        }

        #[unsafe(method(contentSharingPickerStartDidFailWithError:))]
        fn did_fail(&self, error: &NSError) {
            let why = format!("screen capture: {}", error.localizedDescription());
            answer(|| Err(why));
        }
    }
);

impl Observer {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        // SAFETY: `init` on a freshly allocated `NSObject` subclass.
        unsafe { msg_send![super(this), init] }
    }
}

/// What a stream's output and delegate write into.
struct Slots {
    frame: Slot<Arc<Frame>>,
    error: Slot<String>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and this adds no `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "SupersilviaScreenCapture"]
    #[ivars = Slots]
    /// A stream's output and its delegate: each frame into the frame slot, the end into the
    /// error slot.
    struct Capture;

    // SAFETY: `NSObjectProtocol` has no requirements beyond being an object.
    unsafe impl NSObjectProtocol for Capture {}

    // SAFETY: the method has the signature the protocol declares.
    unsafe impl SCStreamOutput for Capture {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn did_output(
            &self,
            _stream: &SCStream,
            sample: &CMSampleBuffer,
            kind: SCStreamOutputType,
        ) {
            if kind != SCStreamOutputType::Screen {
                return;
            }
            if status(sample) == Some(SCFrameStatus::Stopped) {
                self.stopped(None);
                return;
            }
            // SAFETY: a sample the stream handed over, valid for this call; the image buffer
            // comes back retained.
            let Some(image) = (unsafe { sample.image_buffer() }) else {
                return;
            };
            match pixels::frame(image) {
                Ok(frame) => put(&self.ivars().frame, Arc::new(frame)),
                Err(e) => put(&self.ivars().error, e),
            }
        }
    }

    // SAFETY: the method has the signature the protocol declares.
    unsafe impl SCStreamDelegate for Capture {
        // The header declares the error non-null; a nil one has been seen, and is a stop with
        // nothing more to say.
        #[unsafe(method(stream:didStopWithError:))]
        fn did_stop(&self, _stream: &SCStream, error: Option<&NSError>) {
            self.stopped(error);
        }
    }
);

impl Capture {
    fn new(frame: Slot<Arc<Frame>>, error: Slot<String>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Slots { frame, error });
        // SAFETY: `init` on a freshly allocated `NSObject` subclass.
        unsafe { msg_send![super(this), init] }
    }

    /// The stream has ended: say so in the error slot.
    fn stopped(&self, error: Option<&NSError>) {
        put(&self.ivars().error, ended(error));
    }
}

/// What the status line says when a stream ends with `error`: the plain line for the person's
/// own stop or for none, and the system's reason beside it otherwise.
fn ended(error: Option<&NSError>) -> String {
    match error {
        Some(error) if error.code() != USER_STOPPED => {
            format!("the source stopped: {}", error.localizedDescription())
        }
        _ => "the source stopped".to_string(),
    }
}

/// A screen sample's `SCFrameStatus`, from the attachments dictionary ScreenCaptureKit puts on
/// it, or `None` for a sample carrying none.
fn status(sample: &CMSampleBuffer) -> Option<SCFrameStatus> {
    // SAFETY: a sample the stream handed over, valid for this call; asked not to create an
    // array, so it is only read.
    let attachments = unsafe { sample.sample_attachments_array(false) }?;
    // SAFETY: the array holds one dictionary per sample (`CMSampleBuffer.h`), keyed by
    // strings; each value is only read as CoreFoundation's base type.
    let attachments = unsafe { attachments.cast_unchecked::<CFDictionary<CFString, CFType>>() };
    let first = attachments.get(0)?;
    // SAFETY: a constant ScreenCaptureKit exports, an `NSString`, which is toll-free bridged
    // to the `CFString` the dictionary is keyed by.
    let key = unsafe { &*std::ptr::from_ref(SCStreamFrameInfoStatus).cast::<CFString>() };
    let value = first.get(key)?;
    let number = value.downcast_ref::<CFNumber>()?.as_isize()?;
    Some(SCFrameStatus(number))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::{Camera, Source};

    /// **A screen's camera is its slots, with no pipeline**: a frame the capture writes is the
    /// camera's newest, the same `Arc` until another arrives, and the end it writes is the
    /// camera's error. Nothing here asks for a screen.
    #[test]
    fn a_camera_over_a_screen_reads_the_slots_the_capture_writes() {
        let stream = Stream {
            frame: Arc::default(),
            error: Arc::default(),
            description: "a test".to_string(),
        };
        let mut camera = Camera::open(&Source::Screen(stream.clone()), None).expect("adopted");
        assert!(camera.elements().is_empty(), "no pipeline");
        assert_eq!(camera.description, "a test");
        assert!(camera.latest().is_none(), "nothing written yet");

        let frame = Arc::new(Frame::solid(2, 2, [1, 2, 3, 255]));
        put(&stream.frame, Arc::clone(&frame));
        let got = camera.latest().expect("the frame written");
        assert!(Arc::ptr_eq(&got, &frame));
        assert!(Arc::ptr_eq(&camera.latest().expect("kept"), &frame));
        assert_eq!(camera.error(), None);

        put(&stream.error, "the source stopped".to_string());
        assert_eq!(camera.error().as_deref(), Some("the source stopped"));
    }

    /// An `NSError` in ScreenCaptureKit's domain with `code`, as the delegate is handed one.
    fn error(code: isize) -> Retained<NSError> {
        use objc2::ClassType;
        use objc2::runtime::AnyObject;
        let domain =
            objc2_foundation::NSString::from_str("com.apple.ScreenCaptureKit.SCStreamErrorDomain");
        // SAFETY: `errorWithDomain:code:userInfo:` with a string domain and no user info, which
        // the method allows.
        unsafe {
            msg_send![
                NSError::class(),
                errorWithDomain: &*domain,
                code: code,
                userInfo: std::ptr::null::<AnyObject>()
            ]
        }
    }

    /// **A stop reaches the camera's error**: the person's own *Stop sharing*, a stop with a
    /// nil error, and one the system caused each land in the slot a camera over the stream
    /// reads, the last with the system's reason. Nothing here makes a stream.
    #[test]
    fn a_stream_that_ends_says_so_on_the_camera_over_it() {
        let stream = Stream {
            frame: Arc::default(),
            error: Arc::default(),
            description: "a test".to_string(),
        };
        let camera = Camera::open(&Source::Screen(stream.clone()), None).expect("adopted");
        let capture = Capture::new(Arc::clone(&stream.frame), Arc::clone(&stream.error));
        assert_eq!(camera.error(), None);

        capture.stopped(Some(&error(USER_STOPPED)));
        assert_eq!(camera.error().as_deref(), Some("the source stopped"));

        put(&stream.error, String::new());
        capture.stopped(None);
        assert_eq!(camera.error().as_deref(), Some("the source stopped"));

        // `SCStreamErrorSystemStoppedStream`.
        capture.stopped(Some(&error(-3821)));
        let why = camera.error().expect("an error");
        assert!(why.starts_with("the source stopped: "), "{why}");
    }

    /// **A second capture gets a picker of its own**: the picker allows a stream for each ask
    /// waiting and each stream running, so the Main Input capturing a window and a Screen
    /// Capture node asking is two, where the default of one left the node waiting; and it stays
    /// active while a stream runs with nothing asked.
    #[test]
    fn the_picker_allows_a_stream_for_every_ask_and_every_running_stream() {
        let off = Setting {
            active: false,
            maximum: 0,
        };
        assert_eq!(setting(0, 0), off, "nothing asked, nothing running");
        assert_eq!(
            setting(1, 0),
            Setting {
                active: true,
                maximum: 1
            }
        );
        assert_eq!(
            setting(1, 1),
            Setting {
                active: true,
                maximum: 2
            }
        );
        assert_eq!(
            setting(0, 1),
            Setting {
                active: true,
                maximum: 1
            }
        );
        assert_eq!(
            setting(2, 1),
            Setting {
                active: true,
                maximum: 3
            }
        );
    }

    /// **Each answer is the oldest ask's that is still there**: an ask whose `Pending` has
    /// gone is passed over, the next answer goes to the next ask, and with none waiting the
    /// answer is never made — no stream is started for nobody.
    #[test]
    fn each_answer_goes_to_the_oldest_ask_still_waiting() {
        let mut waiting = VecDeque::new();
        let mut pendings = Vec::new();
        for _ in 0..3 {
            let (ask, pending) = mpsc::channel::<u32>();
            waiting.push_back(ask);
            pendings.push(pending);
        }
        drop(pendings.remove(0));

        reply(|| waiting.pop_front(), || 7);
        assert_eq!(pendings[0].try_recv(), Ok(7), "the gone ask passed over");
        assert!(pendings[1].try_recv().is_err());

        reply(|| waiting.pop_front(), || 8);
        assert_eq!(pendings[1].try_recv(), Ok(8));

        let mut made = false;
        reply(
            || waiting.pop_front(),
            || {
                made = true;
                9
            },
        );
        assert!(!made, "no ask waiting, so no answer made");
    }
}
