//! Narrow Carbon/Core Foundation boundary for the macOS Text Input Source
//! (TIS) query used by the P1-M7 IME indicator.
//!
//! This crate isolates the audited `unsafe` FFI calls so that `diary-tui`
//! can keep `#![forbid(unsafe_code)]`. It has no third-party dependency: it
//! links directly against the system Carbon/HIToolbox and Core Foundation
//! frameworks declared in `build.rs`, using only raw `extern "C"` bindings
//! for the handful of calls P1-M7 needs.

#![cfg(target_os = "macos")]
#![allow(unsafe_code)]

mod ffi;

/// The language family of the currently selected keyboard input source, as
/// far as P1-M7 needs to distinguish it. `Other` covers every input source
/// that is neither English nor Korean; Japanese is explicitly out of scope
/// (see AGENTS.md "Language scope").
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputLanguage {
    English,
    Korean,
    Other,
}

/// Queries the currently selected keyboard input source and classifies it.
/// Every Core Foundation object this touches is released before returning,
/// regardless of which branch is taken.
#[must_use]
pub fn current_input_language() -> InputLanguage {
    ffi::current_input_language()
}

pub(crate) fn classify_input_source_id(id: &str) -> InputLanguage {
    if id.contains("Korean") {
        InputLanguage::Korean
    } else if id.starts_with("com.apple.keylayout.") {
        InputLanguage::English
    } else {
        InputLanguage::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_korean_source_ids_classify_as_korean() {
        assert_eq!(
            classify_input_source_id("com.apple.inputmethod.Korean.2SetKorean"),
            InputLanguage::Korean
        );
        assert_eq!(
            classify_input_source_id("com.apple.inputmethod.Korean.3Set390"),
            InputLanguage::Korean
        );
    }

    #[test]
    fn ascii_keyboard_layouts_classify_as_english() {
        assert_eq!(
            classify_input_source_id("com.apple.keylayout.US"),
            InputLanguage::English
        );
        assert_eq!(
            classify_input_source_id("com.apple.keylayout.ABC"),
            InputLanguage::English
        );
    }

    #[test]
    fn an_unrecognized_source_id_classifies_as_other() {
        assert_eq!(
            classify_input_source_id("com.apple.inputmethod.Japanese"),
            InputLanguage::Other
        );
    }

    #[test]
    fn querying_the_real_input_source_never_panics() {
        let _ = current_input_language();
    }
}
