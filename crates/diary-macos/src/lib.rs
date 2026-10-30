//! Narrow Carbon/Core Foundation boundary for the macOS Text Input Source
//! (TIS) query and selected-source notification used by the terminal IME
//! indicator.
//!
//! This crate isolates the audited `unsafe` FFI calls so that `diary-tui`
//! can keep `#![forbid(unsafe_code)]`. It has no third-party dependency: it
//! links directly against the system Carbon/HIToolbox and Core Foundation
//! frameworks through raw `extern "C"` bindings for the handful of calls
//! the IME indicator needs.

#![cfg(target_os = "macos")]
#![allow(unsafe_code)]

mod ffi;

use std::ffi::c_void;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

/// The language family of the currently selected keyboard input source, as
/// far as the IME indicator needs to distinguish it. `Other` covers every input source
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

pub(crate) fn classify_language_tag(tag: &str) -> InputLanguage {
    let primary = tag
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if primary == "ko" {
        InputLanguage::Korean
    } else if primary == "en" {
        InputLanguage::English
    } else {
        InputLanguage::Other
    }
}

const fn encode_language(language: InputLanguage) -> u8 {
    match language {
        InputLanguage::English => 1,
        InputLanguage::Korean => 2,
        InputLanguage::Other => 0,
    }
}

const fn decode_language(value: u8) -> InputLanguage {
    match value {
        1 => InputLanguage::English,
        2 => InputLanguage::Korean,
        _ => InputLanguage::Other,
    }
}

struct NotificationSlot {
    language: AtomicU8,
    generation: AtomicU64,
}

impl NotificationSlot {
    fn new(language: InputLanguage) -> Self {
        Self {
            language: AtomicU8::new(encode_language(language)),
            generation: AtomicU64::new(1),
        }
    }

    fn publish(&self, language: InputLanguage) {
        self.language
            .store(encode_language(language), Ordering::Release);
        self.generation.fetch_add(1, Ordering::Release);
    }

    fn latest(&self) -> (u64, InputLanguage) {
        let generation = self.generation.load(Ordering::Acquire);
        let language = decode_language(self.language.load(Ordering::Acquire));
        (generation, language)
    }
}

/// Failure to start or continue the notification path. The TUI treats every
/// variant identically: it switches to the bounded 500 ms query fallback and
/// keeps editing available.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MonitorError {
    NotMainThread,
    RegistrationUnavailable,
    DeliveryUnavailable,
}

/// Main-thread TIS notification observer.
///
/// Apple registers Core Foundation distributed-notification delivery on the
/// main thread. `poll_update` therefore performs a non-blocking pass of that
/// thread's default run loop. The `Rc` marker deliberately makes this type
/// neither `Send` nor `Sync`, so registration, polling, removal, and callback
/// context lifetime stay on the same main thread.
pub struct InputLanguageMonitor {
    slot: Box<NotificationSlot>,
    seen_generation: u64,
    _main_thread_only: PhantomData<Rc<()>>,
}

impl InputLanguageMonitor {
    pub fn start() -> Result<Self, MonitorError> {
        if !ffi::is_main_thread() {
            return Err(MonitorError::NotMainThread);
        }
        let slot = Box::new(NotificationSlot::new(current_input_language()));
        let observer = (&raw const *slot).cast::<c_void>();
        if !ffi::add_input_source_observer(observer, input_source_changed) {
            return Err(MonitorError::RegistrationUnavailable);
        }
        if ffi::pump_main_run_loop() == ffi::PumpResult::Unavailable {
            ffi::remove_input_source_observer(observer);
            return Err(MonitorError::DeliveryUnavailable);
        }
        Ok(Self {
            slot,
            seen_generation: 0,
            _main_thread_only: PhantomData,
        })
    }

    /// Pumps pending distributed notifications without blocking and returns
    /// only the latest state. Bursts and duplicate notifications are bounded
    /// to one stored state and one monotonically increasing generation.
    pub fn poll_update(&mut self) -> Result<Option<InputLanguage>, MonitorError> {
        if ffi::pump_main_run_loop() == ffi::PumpResult::Unavailable {
            return Err(MonitorError::DeliveryUnavailable);
        }
        let (generation, language) = self.slot.latest();
        if generation == self.seen_generation {
            Ok(None)
        } else {
            self.seen_generation = generation;
            Ok(Some(language))
        }
    }
}

impl Drop for InputLanguageMonitor {
    fn drop(&mut self) {
        let observer = (&raw const *self.slot).cast::<c_void>();
        ffi::remove_input_source_observer(observer);
    }
}

unsafe extern "C" fn input_source_changed(
    _center: *const c_void,
    observer: *mut c_void,
    _name: *const c_void,
    _object: *const c_void,
    _user_info: *const c_void,
) {
    if observer.is_null() {
        return;
    }
    // SAFETY: `InputLanguageMonitor` registers a pointer to its stable boxed
    // slot and unregisters before dropping that box. The monitor is main-thread
    // only, matching Core Foundation's distributed-notification delivery.
    let slot = unsafe { &*observer.cast::<NotificationSlot>() };
    slot.publish(current_input_language());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn korean_language_tags_classify_as_korean() {
        assert_eq!(classify_language_tag("ko"), InputLanguage::Korean);
        assert_eq!(classify_language_tag("ko-KR"), InputLanguage::Korean);
    }

    #[test]
    fn english_language_tags_classify_as_english() {
        assert_eq!(classify_language_tag("en"), InputLanguage::English);
        assert_eq!(classify_language_tag("EN_US"), InputLanguage::English);
    }

    #[test]
    fn a_missing_or_unrecognized_language_classifies_as_other() {
        assert_eq!(classify_language_tag(""), InputLanguage::Other);
        assert_eq!(classify_language_tag("ja"), InputLanguage::Other);
    }

    #[test]
    fn notification_slot_coalesces_a_burst_to_the_latest_state() {
        let slot = NotificationSlot::new(InputLanguage::English);
        slot.publish(InputLanguage::Korean);
        slot.publish(InputLanguage::Korean);
        slot.publish(InputLanguage::English);

        let (generation, language) = slot.latest();
        assert_eq!(generation, 4);
        assert_eq!(language, InputLanguage::English);
    }

    #[test]
    fn querying_the_real_input_source_never_panics() {
        let _ = current_input_language();
    }

    #[test]
    fn real_monitor_starts_polls_and_removes_its_observer() {
        if !ffi::is_main_thread() {
            assert!(matches!(
                InputLanguageMonitor::start(),
                Err(MonitorError::NotMainThread)
            ));
            return;
        }
        let mut monitor = InputLanguageMonitor::start().expect("main-thread monitor should start");
        assert!(
            monitor
                .poll_update()
                .expect("run loop should be available")
                .is_some()
        );
        drop(monitor);
    }
}
