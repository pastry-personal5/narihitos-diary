//! Raw `extern "C"` bindings for the one-shot TIS query. Every `unsafe`
//! block here is narrowly scoped and documented with the specific Apple
//! ownership rule it relies on.

use std::ffi::{CStr, c_char, c_void};

use crate::{InputLanguage, classify_input_source_id};

type CfTypeRef = *const c_void;
type CfStringRef = CfTypeRef;
type TisInputSourceRef = CfTypeRef;
type CfIndex = isize;
type CfStringEncoding = u32;
type Boolean = u8;

const K_CF_STRING_ENCODING_UTF8: CfStringEncoding = 0x0800_0100;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn TISCopyCurrentKeyboardInputSource() -> TisInputSourceRef;
    fn TISGetInputSourceProperty(
        input_source: TisInputSourceRef,
        key: CfStringRef,
    ) -> *const c_void;
    static kTISPropertyInputSourceID: CfStringRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: CfTypeRef);
    fn CFStringGetLength(value: CfStringRef) -> CfIndex;
    fn CFStringGetCString(
        value: CfStringRef,
        buffer: *mut c_char,
        buffer_size: CfIndex,
        encoding: CfStringEncoding,
    ) -> Boolean;
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
    // `TISInputSourceRef`. `kTISPropertyInputSourceID` is a Carbon-owned
    // static `CFStringRef`; `TISGetInputSourceProperty` is a "Get" (not
    // "Copy") call, so its result is not owned by us and must not be
    // released.
    let property = unsafe { TISGetInputSourceProperty(source, kTISPropertyInputSourceID) };

    let language = if property.is_null() {
        InputLanguage::Other
    } else {
        match copy_cfstring_to_utf8(property.cast()) {
            Some(id) => classify_input_source_id(&id),
            None => InputLanguage::Other,
        }
    };

    // SAFETY: `source` is the one owned reference acquired above; this is
    // its single, unconditional release on every return path.
    unsafe { CFRelease(source) };

    language
}

/// Copies a borrowed `CFStringRef` into an owned `String`. Returns `None`
/// rather than truncating silently if the string does not fit the fixed
/// buffer; a real input-source identifier is always short ASCII.
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
