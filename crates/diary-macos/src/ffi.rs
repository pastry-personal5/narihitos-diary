//! Raw `extern "C"` bindings for the TIS query and selected-input-source
//! notification. Every `unsafe` block here is narrowly scoped and documented
//! with the specific Apple ownership or callback rule it relies on.

use std::ffi::{CStr, c_char, c_void};

use crate::InputLanguage;

type CfTypeRef = *const c_void;
type CfStringRef = CfTypeRef;
type CfArrayRef = CfTypeRef;
type CfNotificationCenterRef = *const c_void;
type TisInputSourceRef = CfTypeRef;
type CfIndex = isize;
type CfTypeId = usize;
type CfStringEncoding = u32;
type Boolean = u8;
type CfTimeInterval = f64;
type CfNotificationSuspensionBehavior = CfIndex;

pub(crate) type NotificationCallback = unsafe extern "C" fn(
    center: CfNotificationCenterRef,
    observer: *mut c_void,
    name: CfStringRef,
    object: *const c_void,
    user_info: *const c_void,
);

const K_CF_STRING_ENCODING_UTF8: CfStringEncoding = 0x0800_0100;
const DELIVER_IMMEDIATELY: CfNotificationSuspensionBehavior = 4;
const RUN_LOOP_RUN_FINISHED: i32 = 1;
const RUN_LOOP_RUN_HANDLED_SOURCE: i32 = 4;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn TISCopyCurrentKeyboardInputSource() -> TisInputSourceRef;
    fn TISGetInputSourceProperty(
        input_source: TisInputSourceRef,
        key: CfStringRef,
    ) -> *const c_void;
    static kTISPropertyInputSourceLanguages: CfStringRef;
    static kTISNotifySelectedKeyboardInputSourceChanged: CfStringRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: CfTypeRef);
    fn CFGetTypeID(value: CfTypeRef) -> CfTypeId;
    fn CFArrayGetTypeID() -> CfTypeId;
    fn CFArrayGetCount(array: CfArrayRef) -> CfIndex;
    fn CFArrayGetValueAtIndex(array: CfArrayRef, index: CfIndex) -> *const c_void;
    fn CFStringGetTypeID() -> CfTypeId;
    fn CFStringGetLength(value: CfStringRef) -> CfIndex;
    fn CFStringGetCString(
        value: CfStringRef,
        buffer: *mut c_char,
        buffer_size: CfIndex,
        encoding: CfStringEncoding,
    ) -> Boolean;
    fn CFNotificationCenterGetDistributedCenter() -> CfNotificationCenterRef;
    fn CFNotificationCenterAddObserver(
        center: CfNotificationCenterRef,
        observer: *const c_void,
        callback: NotificationCallback,
        name: CfStringRef,
        object: *const c_void,
        suspension_behavior: CfNotificationSuspensionBehavior,
    );
    fn CFNotificationCenterRemoveObserver(
        center: CfNotificationCenterRef,
        observer: *const c_void,
        name: CfStringRef,
        object: *const c_void,
    );
    fn CFRunLoopRunInMode(
        mode: CfStringRef,
        seconds: CfTimeInterval,
        return_after_source_handled: Boolean,
    ) -> i32;
    static kCFRunLoopDefaultMode: CfStringRef;
}

unsafe extern "C" {
    fn pthread_main_np() -> i32;
}

/// Queries the current keyboard input source once and classifies it.
/// Returns `InputLanguage::Other` for any failure (null source, null
/// property, or a string too long for the fixed-size buffer below) rather
/// than panicking — the caller's 500ms polling fallback keeps running
/// either way.
pub fn current_input_language() -> InputLanguage {
    // SAFETY: `TISCopyCurrentKeyboardInputSource` takes no arguments and, per
    // Apple's "Copy" naming convention, returns a reference this function
    // owns and must release exactly once on every path.
    let source = unsafe { TISCopyCurrentKeyboardInputSource() };
    if source.is_null() {
        return InputLanguage::Other;
    }

    // SAFETY: `source` was just null-checked and is a valid, owned
    // `TISInputSourceRef`. `kTISPropertyInputSourceLanguages` is a Carbon-owned
    // static `CFStringRef`; `TISGetInputSourceProperty` is a "Get" (not
    // "Copy") call, so its result is not owned by us and must not be
    // released.
    let property = unsafe { TISGetInputSourceProperty(source, kTISPropertyInputSourceLanguages) };

    let language = if property.is_null() {
        InputLanguage::Other
    } else {
        copy_primary_language(property.cast()).unwrap_or(InputLanguage::Other)
    };

    // SAFETY: `source` is the one owned reference acquired above; this is
    // its single, unconditional release on every return path.
    unsafe { CFRelease(source) };

    language
}

/// Reads the first BCP 47 language tag from the borrowed TIS languages array.
/// Type checks happen before the Core Foundation array/string accessors so a
/// malformed or unexpected property becomes `Other` instead of invoking an
/// accessor with the wrong opaque type.
fn copy_primary_language(value: CfTypeRef) -> Option<InputLanguage> {
    // SAFETY: `value` is non-null and points at a live property of the owned
    // input source. Type-ID functions only inspect Core Foundation metadata.
    if unsafe { CFGetTypeID(value) } != unsafe { CFArrayGetTypeID() } {
        return None;
    }
    // SAFETY: the type-ID check above proves `value` is a CFArray. The value
    // remains borrowed from the live input source until this function returns.
    let count = unsafe { CFArrayGetCount(value) };
    if count <= 0 {
        return None;
    }
    // SAFETY: index zero is in bounds because `count > 0`; the returned value
    // is borrowed from the array and is not released here.
    let first = unsafe { CFArrayGetValueAtIndex(value, 0) };
    if first.is_null() || unsafe { CFGetTypeID(first) } != unsafe { CFStringGetTypeID() } {
        return None;
    }
    copy_cfstring_to_utf8(first.cast()).map(|tag| crate::classify_language_tag(&tag))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PumpResult {
    HandledSource,
    Idle,
    Unavailable,
}

#[must_use]
pub(crate) fn is_main_thread() -> bool {
    // SAFETY: `pthread_main_np` takes no arguments and has no ownership effect.
    unsafe { pthread_main_np() != 0 }
}

/// Registers the process-global TIS distributed notification. Apple delivers
/// CF distributed notifications on the main thread, so callers verify and
/// pump that thread's run loop through this module.
pub(crate) fn add_input_source_observer(
    observer: *const c_void,
    callback: NotificationCallback,
) -> bool {
    // SAFETY: both functions return process-global borrowed constants. They
    // require no release and are null-checked before registration.
    let center = unsafe { CFNotificationCenterGetDistributedCenter() };
    let name = unsafe { kTISNotifySelectedKeyboardInputSourceChanged };
    if center.is_null() || name.is_null() || observer.is_null() {
        return false;
    }
    // SAFETY: the caller keeps `observer` alive until it calls the matching
    // removal function. The callback has the C ABI required by Core Foundation;
    // object is null to accept this named notification from any source.
    unsafe {
        CFNotificationCenterAddObserver(
            center,
            observer,
            callback,
            name,
            std::ptr::null(),
            DELIVER_IMMEDIATELY,
        );
    }
    true
}

pub(crate) fn remove_input_source_observer(observer: *const c_void) {
    // SAFETY: these are the same process-global borrowed values used for
    // registration. A null guard makes teardown safe after partial startup.
    let center = unsafe { CFNotificationCenterGetDistributedCenter() };
    let name = unsafe { kTISNotifySelectedKeyboardInputSourceChanged };
    if center.is_null() || name.is_null() || observer.is_null() {
        return;
    }
    // SAFETY: `observer` is the exact stable identity used to register, and the
    // caller does not free its context until this removal returns.
    unsafe { CFNotificationCenterRemoveObserver(center, observer, name, std::ptr::null()) };
}

pub(crate) fn pump_main_run_loop() -> PumpResult {
    // SAFETY: the default-mode string is a process-global borrowed constant.
    // A zero timeout performs one non-blocking pass on the calling main thread.
    let mode = unsafe { kCFRunLoopDefaultMode };
    if mode.is_null() {
        return PumpResult::Unavailable;
    }
    let result = unsafe { CFRunLoopRunInMode(mode, 0.0, 1) };
    match result {
        RUN_LOOP_RUN_FINISHED => PumpResult::Unavailable,
        RUN_LOOP_RUN_HANDLED_SOURCE => PumpResult::HandledSource,
        _ => PumpResult::Idle,
    }
}

/// Copies a borrowed `CFStringRef` into an owned `String`. Returns `None`
/// rather than truncating silently if the string does not fit the fixed
/// buffer; a real BCP 47 language tag is always short ASCII.
fn copy_cfstring_to_utf8(value: CfStringRef) -> Option<String> {
    if value.is_null() {
        return None;
    }
    // SAFETY: `value` was null-checked; `CFStringGetLength` only reads the
    // string's existing length and does not transfer ownership.
    let length = unsafe { CFStringGetLength(value) };
    if !(0..=512).contains(&length) {
        return None;
    }
    let mut buffer = vec![0_u8; length as usize + 1];
    // SAFETY: `buffer` is a valid, writable region of the size passed as
    // `buffer_size`; `CFStringGetCString` writes at most that many bytes
    // including the NUL terminator and does not transfer ownership of
    // `value`.
    let ok = unsafe {
        CFStringGetCString(
            value,
            buffer.as_mut_ptr().cast(),
            buffer.len() as CfIndex,
            K_CF_STRING_ENCODING_UTF8,
        )
    };
    if ok == 0 {
        return None;
    }
    // SAFETY: `buffer` was just filled by `CFStringGetCString`, which
    // NUL-terminates on success.
    let c_str = unsafe { CStr::from_ptr(buffer.as_ptr().cast()) };
    c_str.to_str().ok().map(ToOwned::to_owned)
}
