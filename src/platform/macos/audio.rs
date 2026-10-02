// SPDX-License-Identifier: AGPL-3.0-or-later

//! Audio capture by name on macOS: Core Audio lists the inputs, and GStreamer's `osxaudiosrc`
//! captures one by its UID.
//!
//! **GStreamer's device monitor is never asked for audio here.** Over `Audio/Source`,
//! `osxaudiodeviceprovider` does more than list: it binds a HAL AudioUnit to each microphone
//! to probe its formats, which opens it, and from a session that could not show the
//! permission prompt that probe never returned. [`sources`] runs inside the synth's tick, so a
//! stall there stops the picture. Core Audio's properties answer the same question opening
//! nothing: `kAudioHardwarePropertyDevices`, then each device's input streams, name and UID.
//!
//! **A named input is `osxaudiosrc unique-id=<UID>`.** The UID persists across boots, where an
//! `AudioDeviceID` does not, and it is what [`crate::audio::Device::Pulse`] holds on a Mac. The
//! element matches it against the devices it lists before the default or a device ID.
//!
//! **The loopback is a Core Audio process tap**, which macOS 14.2 and later offer: a
//! [`Tap`] over every process's output, supersilvia's included as a PulseAudio monitor
//! includes it on Linux, read by an IO proc on an aggregate device of its own and handed to
//! the analyzer as cpal's callback hands it blocks, with no GStreamer between.
//! [`crate::audio::device::DEFAULT_MONITOR`], the loopback's word in a saved file, is what
//! opens it. It needs only the audio-capture grant, where ScreenCaptureKit's system audio needs
//! a stream over a display and so the screen's grant or its picker. Linux's monitor is what goes
//! to the default output; a global tap is what every process plays, to any output.
//!
//! **The aggregate device holds the tap and nothing else.** Apple's headers ask for no main
//! sub-device, and one would put that device's own input streams beside the tap's, which on
//! an audio interface are its microphones. Nor does it set `tapautostart`: with it, the
//! headers say, `AudioDeviceStart` waits until a tapped process first plays, and the start is
//! on the synth's thread.

use crate::audio::device::DEFAULT_MONITOR;
use crate::platform::audio::Source;
use objc2::AllocAnyThread;
use objc2_core_audio::{
    AudioDeviceCreateIOProcID, AudioDeviceDestroyIOProcID, AudioDeviceIOProcID, AudioDeviceStart,
    AudioDeviceStop, AudioHardwareCreateAggregateDevice, AudioHardwareCreateProcessTap,
    AudioHardwareDestroyAggregateDevice, AudioHardwareDestroyProcessTap,
    AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize, AudioObjectID,
    AudioObjectPropertyAddress, AudioObjectPropertyScope, AudioObjectPropertySelector,
    CATapDescription, CATapMuteBehavior, kAudioAggregateDeviceIsPrivateKey,
    kAudioAggregateDeviceNameKey, kAudioAggregateDeviceTapListKey, kAudioAggregateDeviceUIDKey,
    kAudioDevicePropertyDeviceUID, kAudioDevicePropertyNominalSampleRate,
    kAudioDevicePropertyStreams, kAudioHardwarePropertyDevices, kAudioObjectPropertyElementMain,
    kAudioObjectPropertyName, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyScopeInput,
    kAudioObjectSystemObject, kAudioObjectUnknown, kAudioSubTapDriftCompensationKey,
    kAudioSubTapUIDKey, kAudioTapPropertyFormat,
};
use objc2_core_audio_types::{
    AudioBuffer, AudioBufferList, AudioStreamBasicDescription, AudioTimeStamp,
    kAudioFormatFlagIsFloat, kAudioFormatLinearPCM,
};
use objc2_core_foundation::{CFArray, CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_foundation::{NSArray, NSUUID};
use std::ffi::{CStr, c_void};
use std::ptr::NonNull;

/// What every aggregate device a [`Tap`] makes has its UID begin with, so the list of inputs
/// leaves it out.
const AGGREGATE: &str = "supersilvia-loopback-";

/// Every device with an input stream, in Core Audio's order, by its UID and its name. A
/// loopback's own aggregate device is not one.
pub fn sources() -> Vec<Source> {
    devices()
        .into_iter()
        .filter(|&device| {
            size(
                device,
                address(kAudioDevicePropertyStreams, kAudioObjectPropertyScopeInput),
            )
            .is_some_and(|bytes| bytes > 0)
        })
        .filter_map(|device| {
            let name = string(device, kAudioDevicePropertyDeviceUID)
                .filter(|uid| !uid.starts_with(AGGREGATE))?;
            let label = string(device, kAudioObjectPropertyName).unwrap_or_else(|| name.clone());
            Some(Source {
                name,
                label,
                monitor: false,
            })
        })
        .collect()
}

/// The source element for the input with this UID, up to the conversion.
///
/// `provide-clock=false` and `do-timestamp=true`, as Linux's `pulsesrc`: the pipeline hands
/// blocks to an analyzer as they arrive, and nothing downstream of the sink cares what time it
/// is.
pub fn element(name: &str) -> Result<String, String> {
    Ok(format!(
        "osxaudiosrc unique-id=\"{name}\" provide-clock=false do-timestamp=true"
    ))
}

// ----------------------------------------------------------------------------- loopback

/// The loopback: a process tap over every process, in a private aggregate device, read by an
/// IO proc once [`Tap::start`] has one to call. Dropping it stops the proc, then destroys the
/// aggregate device and the tap.
pub struct Tap {
    /// The process tap.
    source: AudioObjectID,
    device: AudioObjectID,
    rate: u32,
    /// The IO proc, once started.
    proc_id: AudioDeviceIOProcID,
    /// What the IO proc reads through its client pointer, from [`Box::leak`]; freed once the
    /// proc is destroyed.
    reader: Option<NonNull<Reader>>,
}

// SAFETY: `reader` is touched by the IO proc and, once the proc is destroyed, by `Drop`; the
// Core Audio objects are IDs any thread may use.
unsafe impl Send for Tap {}

/// The IO proc's own state: the mono block, grown once, and the analyzer's closure.
struct Reader {
    mono: Vec<f32>,
    analyze: Analyze,
}

/// The analyzer's closure, as the IO proc holds it.
type Analyze = Box<dyn FnMut(&[f32]) + Send>;

impl Tap {
    /// The tap for [`DEFAULT_MONITOR`], made and not yet running; `None` for any other name,
    /// which is an input GStreamer opens.
    pub fn open(name: &str) -> Option<Result<Self, String>> {
        (name == DEFAULT_MONITOR).then(Self::make)
    }

    /// The rate the IO proc hands samples over at.
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Start the IO proc, which mixes each block to mono and hands it to `analyze` on Core
    /// Audio's real-time thread.
    pub fn start(&mut self, analyze: impl FnMut(&[f32]) + Send + 'static) -> Result<(), String> {
        let reader = NonNull::from(Box::leak(Box::new(Reader {
            // A second of mono, which no IO cycle reaches.
            mono: Vec::with_capacity(self.rate as usize),
            analyze: Box::new(analyze),
        })));
        self.reader = Some(reader);
        let mut proc_id: AudioDeviceIOProcID = None;
        // SAFETY: `io_proc` matches `AudioDeviceIOProc`, and `reader` lives until `Drop` has
        // destroyed the proc.
        let status = unsafe {
            AudioDeviceCreateIOProcID(
                self.device,
                Some(io_proc),
                reader.as_ptr().cast(),
                NonNull::from(&mut proc_id),
            )
        };
        check(status, "the loopback's reader")?;
        self.proc_id = proc_id;
        // SAFETY: the proc was made on this device just above.
        let status = unsafe { AudioDeviceStart(self.device, self.proc_id) };
        check(status, "starting the loopback")
    }

    /// The tap, its aggregate device and its rate. What is made before a failure is torn down
    /// by `Drop`.
    fn make() -> Result<Self, String> {
        let mut made = Self {
            source: kAudioObjectUnknown,
            device: kAudioObjectUnknown,
            rate: 0,
            proc_id: None,
            reader: None,
        };
        // SAFETY: `alloc` is paired with the description's own initializer.
        let description = unsafe {
            CATapDescription::initStereoGlobalTapButExcludeProcesses(
                CATapDescription::alloc(),
                &NSArray::new(),
            )
        };
        // SAFETY: plain setters on a description nothing else holds.
        unsafe {
            description.setPrivate(true);
            description.setMuteBehavior(CATapMuteBehavior::Unmuted);
        }
        // SAFETY: `description` is a live description, and `made.source` outlives the call.
        let status =
            unsafe { AudioHardwareCreateProcessTap(Some(&description), &raw mut made.source) };
        check(status, "the system audio tap")?;
        // SAFETY: a getter on the same description.
        let tap_uid = unsafe { description.UUID() }.UUIDString().to_string();

        let key = |k: &CStr| CFString::from_str(&k.to_string_lossy());
        let yes = CFNumber::new_i32(1);
        let tap_entry = CFDictionary::<CFString, CFType>::from_slices(
            &[
                &*key(kAudioSubTapUIDKey),
                &*key(kAudioSubTapDriftCompensationKey),
            ],
            &[cf(&*CFString::from_str(&tap_uid)), cf(&*yes)],
        );
        let taps = CFArray::from_objects(&[&*tap_entry]);
        let uid = format!("{AGGREGATE}{}", NSUUID::new().UUIDString());
        let composition = CFDictionary::<CFString, CFType>::from_slices(
            &[
                &*key(kAudioAggregateDeviceNameKey),
                &*key(kAudioAggregateDeviceUIDKey),
                &*key(kAudioAggregateDeviceIsPrivateKey),
                &*key(kAudioAggregateDeviceTapListKey),
            ],
            &[
                cf(&*CFString::from_str("supersilvia loopback")),
                cf(&*CFString::from_str(&uid)),
                cf(&*yes),
                cf(&*taps),
            ],
        );
        // SAFETY: the composition's keys and values are the types the headers give each key,
        // and `made.device` outlives the call.
        let status = unsafe {
            AudioHardwareCreateAggregateDevice(
                composition.as_opaque(),
                NonNull::from(&mut made.device),
            )
        };
        check(status, "the system audio device")?;

        // SAFETY: the tap's format is an `AudioStreamBasicDescription`.
        let format: AudioStreamBasicDescription = unsafe {
            value(
                made.source,
                address(kAudioTapPropertyFormat, kAudioObjectPropertyScopeGlobal),
            )
        }
        .ok_or("the system audio tap has no format")?;
        if format.mFormatID != kAudioFormatLinearPCM
            || format.mFormatFlags & kAudioFormatFlagIsFloat == 0
            || format.mBitsPerChannel != 32
        {
            return Err("the system audio tap is not 32-bit float".to_string());
        }
        // The device's own rate is the one its IO cycles run at; the tap's is the fallback.
        // SAFETY: a device's nominal rate is an `f64`.
        let nominal: Option<f64> = unsafe {
            value(
                made.device,
                address(
                    kAudioDevicePropertyNominalSampleRate,
                    kAudioObjectPropertyScopeGlobal,
                ),
            )
        };
        made.rate = nominal.unwrap_or(format.mSampleRate).round() as u32;
        if made.rate == 0 {
            return Err("the system audio tap has no rate".to_string());
        }
        Ok(made)
    }
}

impl Drop for Tap {
    fn drop(&mut self) {
        // Each call is made on what this tap made, in the reverse order, and a failure leaves
        // nothing more to undo.
        if self.proc_id.is_some() {
            // SAFETY: the proc was made on this device. Stopped from outside the IO thread and
            // destroyed, it is not called again, so the reader is not read after this.
            unsafe {
                AudioDeviceStop(self.device, self.proc_id);
                AudioDeviceDestroyIOProcID(self.device, self.proc_id);
            }
        }
        if let Some(reader) = self.reader.take() {
            // SAFETY: from `Box::leak` in `start`, and no proc reads it any more.
            drop(unsafe { Box::from_raw(reader.as_ptr()) });
        }
        if self.device != kAudioObjectUnknown {
            // SAFETY: an aggregate device this tap made.
            unsafe { AudioHardwareDestroyAggregateDevice(self.device) };
        }
        if self.source != kAudioObjectUnknown {
            // SAFETY: a process tap this tap made.
            unsafe { AudioHardwareDestroyProcessTap(self.source) };
        }
    }
}

/// The IO proc: every input buffer of the cycle, mixed to mono and handed to the analyzer.
unsafe extern "C-unwind" fn io_proc(
    _device: AudioObjectID,
    _now: NonNull<AudioTimeStamp>,
    input: NonNull<AudioBufferList>,
    _input_time: NonNull<AudioTimeStamp>,
    _output: NonNull<AudioBufferList>,
    _output_time: NonNull<AudioTimeStamp>,
    client: *mut c_void,
) -> i32 {
    // SAFETY: `client` is the `Reader` `start` registered, alive until the proc is destroyed,
    // and Core Audio calls one proc one cycle at a time.
    let reader = unsafe { &mut *client.cast::<Reader>() };
    // SAFETY: an `AudioBufferList` holds `mNumberBuffers` buffers in a row, from `mBuffers`.
    let buffers: &[AudioBuffer] = unsafe {
        let list = input.as_ptr();
        std::slice::from_raw_parts(
            (&raw const (*list).mBuffers).cast::<AudioBuffer>(),
            (*list).mNumberBuffers as usize,
        )
    };
    let blocks = buffers.iter().filter_map(|b| {
        let data = NonNull::new(b.mData.cast::<f32>())?;
        let len = b.mDataByteSize as usize / size_of::<f32>();
        // SAFETY: an enabled buffer holds `mDataByteSize` bytes of the tap's 32-bit floats.
        let samples = unsafe { std::slice::from_raw_parts(data.as_ptr(), len) };
        Some((samples, b.mNumberChannels as usize))
    });
    mixdown(blocks, &mut reader.mono);
    if reader.mono.is_empty() {
        return 0;
    }
    // A panic stops at the edge of Core Audio's thread rather than unwinding into it.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        (reader.analyze)(&reader.mono);
    }));
    0
}

/// Mix one IO cycle's blocks, each interleaved samples and its channel count, to mono in
/// `mono`: every channel of every block summed and divided by how many there are. `mono` is
/// cleared first and holds the shortest block's frames; it allocates only to grow.
fn mixdown<'a>(blocks: impl Iterator<Item = (&'a [f32], usize)> + Clone, mono: &mut Vec<f32>) {
    let blocks = blocks.filter(|(_, channels)| *channels > 0);
    let frames = blocks
        .clone()
        .map(|(samples, channels)| samples.len() / channels)
        .min()
        .unwrap_or(0);
    let channels: usize = blocks.clone().map(|(_, channels)| channels).sum();
    mono.clear();
    mono.resize(frames, 0.0);
    for (samples, ch) in blocks {
        for (sum, frame) in mono.iter_mut().zip(samples.chunks_exact(ch)) {
            *sum += frame.iter().sum::<f32>();
        }
    }
    let scale = 1.0 / channels.max(1) as f32;
    for sample in mono.iter_mut() {
        *sample *= scale;
    }
}

/// An `OSStatus` as a result, naming what failed.
fn check(status: i32, what: &str) -> Result<(), String> {
    if status == 0 {
        Ok(())
    } else {
        Err(format!("{what}: Core Audio error {}", fourcc(status)))
    }
}

/// An `OSStatus` as Core Audio's four characters where it is printable, else as a number.
fn fourcc(status: i32) -> String {
    let bytes = status.to_be_bytes();
    if bytes.iter().all(|b| b.is_ascii_graphic() || *b == b' ') {
        format!("'{}'", String::from_utf8_lossy(&bytes))
    } else {
        status.to_string()
    }
}

/// A property of the object as a whole, or of one scope of it.
fn address(
    selector: AudioObjectPropertySelector,
    scope: AudioObjectPropertyScope,
) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: scope,
        mElement: kAudioObjectPropertyElementMain,
    }
}

/// How many bytes a property's value takes, or `None` where the object has no such property.
fn size(object: AudioObjectID, address: AudioObjectPropertyAddress) -> Option<u32> {
    let mut bytes = 0u32;
    // SAFETY: `address` and `bytes` outlive the call, and there is no qualifier.
    let status = unsafe {
        AudioObjectGetPropertyDataSize(
            object,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut bytes),
        )
    };
    (status == 0).then_some(bytes)
}

/// Read a property into `out`, which is `bytes` long, and answer how many bytes it wrote.
///
/// # Safety
///
/// `out` must be valid for writes of `bytes` bytes, and the property's value must be plain
/// data of the type `out` points at.
unsafe fn read(
    object: AudioObjectID,
    address: AudioObjectPropertyAddress,
    bytes: u32,
    out: NonNull<c_void>,
) -> Option<u32> {
    let mut bytes = bytes;
    // SAFETY: the caller's contract covers `out`; `address` and `bytes` outlive the call.
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut bytes),
            out,
        )
    };
    (status == 0).then_some(bytes)
}

/// A property whose value is one `T`.
///
/// # Safety
///
/// The property's value must be plain data of type `T`.
unsafe fn value<T: Copy>(object: AudioObjectID, address: AudioObjectPropertyAddress) -> Option<T> {
    let mut out = std::mem::MaybeUninit::<T>::uninit();
    let bytes = size_of::<T>() as u32;
    // SAFETY: `out` has room for a `T`, and the caller says the value is one.
    let wrote = unsafe { read(object, address, bytes, NonNull::from(&mut out).cast()) }?;
    // SAFETY: Core Audio wrote the whole `T`.
    (wrote == bytes).then(|| unsafe { out.assume_init() })
}

/// A Core Foundation object as the `CFType` a dictionary's values are.
fn cf<T: AsRef<CFType> + ?Sized>(object: &T) -> &CFType {
    object.as_ref()
}

/// Every audio device the system object lists.
fn devices() -> Vec<AudioObjectID> {
    let system = kAudioObjectSystemObject as AudioObjectID;
    let list = address(
        kAudioHardwarePropertyDevices,
        kAudioObjectPropertyScopeGlobal,
    );
    let one = size_of::<AudioObjectID>() as u32;
    let Some(bytes) = size(system, list).filter(|&b| b >= one) else {
        return Vec::new();
    };
    let mut ids: Vec<AudioObjectID> = vec![0; (bytes / one) as usize];
    let bytes = ids.len() as u32 * one;
    // SAFETY: `ids` holds `bytes` bytes of `AudioObjectID`s, the property's own type.
    let wrote = unsafe {
        read(
            system,
            list,
            bytes,
            NonNull::from(ids.as_mut_slice()).cast(),
        )
    };
    // The list can shrink between the two calls, when a device goes away.
    ids.truncate((wrote.unwrap_or(0) / one) as usize);
    ids
}

/// A device's `CFString` property, as a `String`.
fn string(device: AudioObjectID, selector: AudioObjectPropertySelector) -> Option<String> {
    let mut value: *const CFString = std::ptr::null();
    let bytes = size_of::<*const CFString>() as u32;
    // SAFETY: the name and the UID are a `CFStringRef` each, which `value` has room for.
    unsafe {
        read(
            device,
            address(selector, kAudioObjectPropertyScopeGlobal),
            bytes,
            NonNull::from(&mut value).cast(),
        )
    }?;
    // SAFETY: Core Audio hands the string over retained, and the caller releases it, which
    // `CFRetained` does on drop.
    let value = unsafe { CFRetained::from_raw(NonNull::new(value.cast_mut())?) };
    Some(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gstreamer::prelude::*;

    /// Only prints. Which inputs exist is a fact about the machine; listing them opens none.
    #[test]
    fn sources_can_be_listed() {
        for source in sources() {
            println!("{}: {}", source.name, source.label);
        }
    }

    /// Every channel of every block counts once, and the block is as long as its frames.
    #[test]
    fn a_cycle_mixes_down_to_mono() {
        let mut mono = Vec::with_capacity(8);
        let stereo = [1.0, 0.0, 0.5, 0.5, -1.0, 1.0];
        mixdown([(&stereo[..], 2)].into_iter(), &mut mono);
        assert_eq!(mono, [0.5, 0.5, 0.0]);

        // Two buffers of one channel each, as a non-interleaved stream arrives.
        let (left, right) = ([1.0, 1.0], [0.0, -1.0]);
        mixdown([(&left[..], 1), (&right[..], 1)].into_iter(), &mut mono);
        assert_eq!(mono, [0.5, 0.0]);

        // Nothing enabled is an empty block, not a division by zero.
        mixdown(std::iter::empty(), &mut mono);
        assert!(mono.is_empty());
        mixdown([(&stereo[..], 0)].into_iter(), &mut mono);
        assert!(mono.is_empty());
        assert_eq!(mono.capacity(), 8, "grown once, never shrunk");
    }

    /// Only the loopback's word opens a tap; every other name is GStreamer's to open. Nothing
    /// here makes a tap.
    #[test]
    fn only_the_loopback_is_a_tap() {
        assert!(Tap::open("BuiltInMicrophoneDevice").is_none());
        assert!(Tap::open("").is_none());
    }

    /// A named input is `osxaudiosrc` by its UID, and GStreamer parses the line.
    #[test]
    fn a_named_input_is_osxaudiosrc_by_its_uid() {
        let line = element("BuiltInMicrophoneDevice").unwrap();
        assert_eq!(
            line,
            "osxaudiosrc unique-id=\"BuiltInMicrophoneDevice\" provide-clock=false \
             do-timestamp=true"
        );
        gstreamer::init().unwrap();
        // Parsed in the null state: built, never started.
        let src = gstreamer::parse::launch(&line).unwrap();
        assert_eq!(
            src.property::<Option<String>>("unique-id").as_deref(),
            Some("BuiltInMicrophoneDevice")
        );
    }
}
