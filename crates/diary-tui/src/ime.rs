//! IME state for the Status Line `IME: EN` / `IME: KO` / `IME: ?` indicator.
//!
//! P1-M7 prefers a TIS notification push (added once the terminal owner's
//! run-loop integration lands); this module provides the always-available
//! 500ms polling fallback and the fakeable boundary ViewModel tests use
//! instead of the real `diary-macos` FFI.

use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImeState {
    English,
    Korean,
    Unknown,
}

impl ImeState {
    #[must_use]
    pub const fn status_label(self) -> &'static str {
        match self {
            Self::English => "IME: EN",
            Self::Korean => "IME: KO",
            Self::Unknown => "IME: ?",
        }
    }
}

pub trait InputLanguageQuery {
    fn query(&self) -> ImeState;
}

#[cfg(target_os = "macos")]
pub struct MacInputLanguageQuery;

#[cfg(target_os = "macos")]
impl InputLanguageQuery for MacInputLanguageQuery {
    fn query(&self) -> ImeState {
        match diary_macos::current_input_language() {
            diary_macos::InputLanguage::English => ImeState::English,
            diary_macos::InputLanguage::Korean => ImeState::Korean,
            diary_macos::InputLanguage::Other => ImeState::Unknown,
        }
    }
}

const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Polls `InputLanguageQuery::query` at most once per 500ms. Any failure of
/// the push-notification path selects this without disabling editing; it
/// never blocks on the underlying query.
pub struct PollingImeTracker<Q: InputLanguageQuery> {
    query: Q,
    last_poll: Option<Instant>,
    current: ImeState,
}

impl<Q: InputLanguageQuery> PollingImeTracker<Q> {
    #[must_use]
    pub const fn new(query: Q) -> Self {
        Self {
            query,
            last_poll: None,
            current: ImeState::Unknown,
        }
    }

    /// Re-queries if at least one poll interval has passed since the last
    /// query, then returns the current best-known state either way.
    pub fn tick(&mut self, now: Instant) -> ImeState {
        let due = match self.last_poll {
            None => true,
            Some(last) => now.duration_since(last) >= POLL_INTERVAL,
        };
        if due {
            self.current = self.query.query();
            self.last_poll = Some(now);
        }
        self.current
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    struct FakeQuery {
        state: Cell<ImeState>,
        calls: Cell<u32>,
    }

    impl InputLanguageQuery for FakeQuery {
        fn query(&self) -> ImeState {
            self.calls.set(self.calls.get() + 1);
            self.state.get()
        }
    }

    #[test]
    fn the_first_tick_always_queries() {
        let fake = FakeQuery {
            state: Cell::new(ImeState::Korean),
            calls: Cell::new(0),
        };
        let mut tracker = PollingImeTracker::new(fake);
        let state = tracker.tick(Instant::now());
        assert_eq!(state, ImeState::Korean);
        assert_eq!(tracker.query.calls.get(), 1);
    }

    #[test]
    fn a_tick_before_the_interval_elapses_does_not_requery() {
        let fake = FakeQuery {
            state: Cell::new(ImeState::English),
            calls: Cell::new(0),
        };
        let mut tracker = PollingImeTracker::new(fake);
        let start = Instant::now();
        tracker.tick(start);
        tracker.query.state.set(ImeState::Korean);
        let state = tracker.tick(start + Duration::from_millis(100));
        assert_eq!(state, ImeState::English, "stale poll must not re-query yet");
        assert_eq!(tracker.query.calls.get(), 1);
    }

    #[test]
    fn a_tick_after_the_interval_elapses_requeries() {
        let fake = FakeQuery {
            state: Cell::new(ImeState::English),
            calls: Cell::new(0),
        };
        let mut tracker = PollingImeTracker::new(fake);
        let start = Instant::now();
        tracker.tick(start);
        tracker.query.state.set(ImeState::Korean);
        let state = tracker.tick(start + Duration::from_millis(500));
        assert_eq!(state, ImeState::Korean);
        assert_eq!(tracker.query.calls.get(), 2);
    }

    #[test]
    fn status_labels_match_the_fixed_indicator_strings() {
        assert_eq!(ImeState::English.status_label(), "IME: EN");
        assert_eq!(ImeState::Korean.status_label(), "IME: KO");
        assert_eq!(ImeState::Unknown.status_label(), "IME: ?");
    }
}
