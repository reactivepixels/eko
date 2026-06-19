//! Minimal CoreAudio HAL access (macOS) for true bit-perfect output.
//!
//! # Role in the one-engine architecture
//!
//! This module is the macOS-only "HAL shim" that sits between the engine and the
//! operating system. It is compiled only on `target_os = "macos"` (via `#![cfg(…)]`)
//! and is called exactly once per track, before the cpal stream opens.
//!
//! # Bit-perfect philosophy
//!
//! cpal opens a stream at the file's own sample rate (e.g. 96 000 Hz), but macOS
//! operates every audio device at a single *nominal* rate. If the stream rate and the
//! device rate differ, the CoreAudio HAL silently inserts a sample-rate converter —
//! and the output is no longer bit-perfect even if every other part of the signal path
//! is clean.
//!
//! The fix, used by both Roon and Audirvana, is to set the device's nominal rate to
//! match the file *before* the cpal stream opens. This module does exactly that via the
//! public `AudioObjectSetPropertyData` HAL call on
//! `kAudioDevicePropertyNominalSampleRate`.
//!
//! After the switch the actual rate is read back and returned so that the engine's
//! `Shared::dev_rate` field can tell the truth: if the device doesn't support the
//! requested rate (e.g. a Bluetooth headset locked to 48 kHz) the UI badge correctly
//! flags "not bit-perfect" rather than silently lying.
//!
//! # Safety and FFI notes
//!
//! All CoreAudio HAL calls are `unsafe` because they cross the C ABI boundary.
//! The code follows the CoreAudio HAL documentation exactly: property addresses are
//! stack-allocated `AudioObjectPropertyAddress` structs, buffer sizes are passed as
//! `*mut u32`, and `CFStringRef` values are released with `CFRelease` immediately
//! after use. No CoreAudio objects are retained across call boundaries.
//!
//! This module avoids the `objc`/`core-foundation` crates to keep the dependency tree
//! minimal — the subset of the HAL needed here is small enough to bind by hand.

#![cfg(target_os = "macos")]

use std::os::raw::c_void;
use std::ptr::null;

/// Opaque 32-bit identifier for a CoreAudio HAL object (device, stream, system, etc.).
type AudioObjectID = u32;

/// CoreAudio/CoreFoundation status code. `0` = `noErr`; any other value is an error.
type OSStatus = i32;

/// Opaque pointer to a CoreFoundation string. Must be released with `CFRelease`.
type CFStringRef = *const c_void;

/// CoreFoundation signed integer type (pointer-sized).
type CFIndex = isize;

/// Mirrors `AudioObjectPropertyAddress` from `<CoreAudio/AudioHardwareBase.h>`.
///
/// Identifies a specific HAL property via a (selector, scope, element) triple.
/// All three fields are FourCC codes encoded as big-endian `u32` values.
#[repr(C)]
struct AudioObjectPropertyAddress {
    /// The property being addressed (e.g. `kAudioDevicePropertyNominalSampleRate`).
    selector: u32,
    /// The scope within the object (e.g. `kAudioObjectPropertyScopeGlobal`).
    scope: u32,
    /// The element within the scope (`0` = master / `kAudioObjectPropertyElementMain`).
    element: u32,
}

/// Mirrors `AudioValueRange` from `<CoreAudio/AudioHardwareBase.h>`.
///
/// Used by `kAudioDevicePropertyAvailableNominalSampleRates` to express a continuous
/// range of supported rates. Most real devices report singleton ranges (min == max).
#[repr(C)]
struct AudioValueRange {
    /// Minimum value of the range (Hz for sample-rate properties).
    minimum: f64,
    /// Maximum value of the range (Hz for sample-rate properties).
    maximum: f64,
}

#[link(name = "CoreAudio", kind = "framework")]
extern "C" {
    fn AudioObjectGetPropertyDataSize(
        obj: AudioObjectID,
        addr: *const AudioObjectPropertyAddress,
        qsize: u32,
        qdata: *const c_void,
        out: *mut u32,
    ) -> OSStatus;
    fn AudioObjectGetPropertyData(
        obj: AudioObjectID,
        addr: *const AudioObjectPropertyAddress,
        qsize: u32,
        qdata: *const c_void,
        iosize: *mut u32,
        out: *mut c_void,
    ) -> OSStatus;
    fn AudioObjectSetPropertyData(
        obj: AudioObjectID,
        addr: *const AudioObjectPropertyAddress,
        qsize: u32,
        qdata: *const c_void,
        size: u32,
        data: *const c_void,
    ) -> OSStatus;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFStringGetCString(s: CFStringRef, buf: *mut u8, size: CFIndex, encoding: u32) -> u8;
    fn CFRelease(cf: *const c_void);
}

/// `kAudioObjectSystemObject` — the singleton HAL system object used to enumerate
/// devices and query global properties.
const SYSTEM_OBJECT: AudioObjectID = 1;

/// `kCFStringEncodingUTF8` — encoding constant passed to `CFStringGetCString`.
const UTF8: u32 = 0x0800_0100;

/// Convert a 4-byte ASCII literal to a big-endian `u32` FourCC, matching the
/// encoding CoreAudio uses for property selectors and scopes.
const fn fourcc(s: &[u8; 4]) -> u32 {
    ((s[0] as u32) << 24) | ((s[1] as u32) << 16) | ((s[2] as u32) << 8) | (s[3] as u32)
}

/// Construct an [`AudioObjectPropertyAddress`] from ASCII FourCC literals.
///
/// `element` is always `0` (`kAudioObjectPropertyElementMain`) — none of the
/// properties queried here are per-element.
fn addr(selector: &[u8; 4], scope: &[u8; 4]) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        selector: fourcc(selector),
        scope: fourcc(scope),
        element: 0,
    }
}

/// Return the HAL object ID of the current system default output device
/// (`kAudioHardwarePropertyDefaultOutputDevice`), or `None` on failure.
fn default_output_device() -> Option<AudioObjectID> {
    let a = addr(b"dOut", b"glob"); // kAudioHardwarePropertyDefaultOutputDevice
    let mut dev: AudioObjectID = 0;
    let mut size = 4u32;
    let st = unsafe {
        AudioObjectGetPropertyData(
            SYSTEM_OBJECT,
            &a,
            0,
            null(),
            &mut size,
            &mut dev as *mut _ as *mut c_void,
        )
    };
    if st == 0 && dev != 0 {
        Some(dev)
    } else {
        None
    }
}

/// Return `true` if `dev` has at least one output stream
/// (`kAudioDevicePropertyStreams` in the output scope).
///
/// Used to filter out input-only and aggregate-internal devices when enumerating
/// the full device list.
fn has_output(dev: AudioObjectID) -> bool {
    let a = addr(b"stm#", b"outp"); // kAudioDevicePropertyStreams, output scope
    let mut size = 0u32;
    let st = unsafe { AudioObjectGetPropertyDataSize(dev, &a, 0, null(), &mut size) };
    st == 0 && size > 0
}

/// Return the localised display name of `dev` (`kAudioObjectPropertyName`), or `None`
/// if the HAL call fails or the CF string can't be converted to UTF-8.
///
/// The `CFStringRef` returned by the HAL is a retained CF object; it is released
/// with `CFRelease` before this function returns.
fn device_name(dev: AudioObjectID) -> Option<String> {
    let a = addr(b"lnam", b"glob"); // kAudioObjectPropertyName
    let mut cfstr: CFStringRef = null();
    let mut size = std::mem::size_of::<CFStringRef>() as u32;
    let st = unsafe {
        AudioObjectGetPropertyData(
            dev,
            &a,
            0,
            null(),
            &mut size,
            &mut cfstr as *mut _ as *mut c_void,
        )
    };
    if st != 0 || cfstr.is_null() {
        return None;
    }
    let mut buf = [0u8; 256];
    let ok = unsafe { CFStringGetCString(cfstr, buf.as_mut_ptr(), buf.len() as CFIndex, UTF8) };
    unsafe { CFRelease(cfstr) };
    if ok == 0 {
        return None;
    }
    let cstr = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr() as *const i8) };
    cstr.to_str().ok().map(|s| s.to_string())
}

/// Find the first output device whose display name matches `name` exactly.
///
/// Enumerates all HAL devices (`kAudioHardwarePropertyDevices`), filters to those
/// with output streams, and returns the first whose name matches. Returns `None`
/// if no device matches (caller falls back to the system default).
fn find_output_device(name: &str) -> Option<AudioObjectID> {
    let a = addr(b"dev#", b"glob"); // kAudioHardwarePropertyDevices
    let mut size = 0u32;
    if unsafe { AudioObjectGetPropertyDataSize(SYSTEM_OBJECT, &a, 0, null(), &mut size) } != 0
        || size == 0
    {
        return None;
    }
    let count = size as usize / std::mem::size_of::<AudioObjectID>();
    let mut devs = vec![0 as AudioObjectID; count];
    if unsafe {
        AudioObjectGetPropertyData(
            SYSTEM_OBJECT,
            &a,
            0,
            null(),
            &mut size,
            devs.as_mut_ptr() as *mut c_void,
        )
    } != 0
    {
        return None;
    }
    devs.into_iter()
        .find(|&d| has_output(d) && device_name(d).as_deref() == Some(name))
}

/// Read the device's current nominal sample rate in Hz
/// (`kAudioDevicePropertyNominalSampleRate`). Returns `None` on HAL error.
fn nominal_rate(dev: AudioObjectID) -> Option<f64> {
    let a = addr(b"nsrt", b"glob"); // kAudioDevicePropertyNominalSampleRate
    let mut rate = 0f64;
    let mut size = 8u32;
    let st = unsafe {
        AudioObjectGetPropertyData(
            dev,
            &a,
            0,
            null(),
            &mut size,
            &mut rate as *mut _ as *mut c_void,
        )
    };
    if st == 0 && rate > 0.0 {
        Some(rate)
    } else {
        None
    }
}

/// Return `true` if `dev` lists `rate` (within ±1 Hz) as a supported nominal rate
/// (`kAudioDevicePropertyAvailableNominalSampleRates`).
///
/// The ±1 Hz tolerance handles the rare case where a device reports
/// `{ min: 44099.0, max: 44101.0 }` rather than the exact integer.
fn supports_rate(dev: AudioObjectID, rate: f64) -> bool {
    let a = addr(b"nsr#", b"glob"); // kAudioDevicePropertyAvailableNominalSampleRates
    let mut size = 0u32;
    if unsafe { AudioObjectGetPropertyDataSize(dev, &a, 0, null(), &mut size) } != 0 || size == 0 {
        return false;
    }
    let count = size as usize / std::mem::size_of::<AudioValueRange>();
    let mut ranges: Vec<AudioValueRange> = (0..count)
        .map(|_| AudioValueRange {
            minimum: 0.0,
            maximum: 0.0,
        })
        .collect();
    if unsafe {
        AudioObjectGetPropertyData(
            dev,
            &a,
            0,
            null(),
            &mut size,
            ranges.as_mut_ptr() as *mut c_void,
        )
    } != 0
    {
        return false;
    }
    ranges
        .iter()
        .any(|r| rate >= r.minimum - 1.0 && rate <= r.maximum + 1.0)
}

/// Attempt to set `dev`'s nominal rate to `rate` Hz via
/// `kAudioDevicePropertyNominalSampleRate`.
///
/// Checks [`supports_rate`] first to avoid a HAL error on unsupported rates.
/// Returns `true` if the HAL call succeeded, `false` if the rate is unsupported
/// or the set call returned a non-zero status.
fn set_nominal_rate(dev: AudioObjectID, rate: f64) -> bool {
    if !supports_rate(dev, rate) {
        return false;
    }
    let a = addr(b"nsrt", b"glob");
    unsafe {
        AudioObjectSetPropertyData(dev, &a, 0, null(), 8, &rate as *const _ as *const c_void) == 0
    }
}

/// Switch the named output device to `rate` Hz so macOS doesn't resample, then read the
/// actual rate back.
///
/// # Arguments
///
/// - `device_name` — the cpal device name string (from `device.name()`), or `None` to
///   target the system default output device.
/// - `rate`        — the file's native sample rate in Hz.
///
/// # Returns
///
/// The device's actual nominal rate (in Hz, rounded to the nearest integer) after the
/// attempted switch, or `None` if no output device could be found.
///
/// When the return value equals `rate` the switch succeeded and macOS will not
/// resample. When it differs (e.g. a Bluetooth device locked to 48 000 Hz when the
/// file is 96 000 Hz) the engine stores the mismatch in [`crate::engine::EngineStatus::dev_rate`]
/// and the UI badge marks the signal path as "not bit-perfect".
///
/// A 60 ms sleep is inserted after a successful rate change to give CoreAudio time to
/// stabilise before the cpal stream opens. This is the same approach used by Roon.
pub fn match_device_rate(device_name: Option<&str>, rate: u32) -> Option<u32> {
    let dev = match device_name {
        Some(n) => find_output_device(n).or_else(default_output_device)?,
        None => default_output_device()?,
    };
    let target = rate as f64;
    let needs = nominal_rate(dev)
        .map(|cur| (cur - target).abs() > 1.0)
        .unwrap_or(true);
    if needs && set_nominal_rate(dev, target) {
        // Give CoreAudio a moment to apply the rate change before playback starts.
        std::thread::sleep(std::time::Duration::from_millis(60));
    }
    nominal_rate(dev).map(|r| r.round() as u32)
}
