//! IME state for the Status Line `IME: EN` / `IME: KO` / `IME: ?` indicator.
//!
//! P1-M7 prefers a TIS notification push on the main run loop and uses the
//! bounded 500 ms query tracker when registration or delivery is unavailable.
//! Both boundaries are fakeable so ViewModel tests never depend on the live
//! input source.

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NotificationUnavailable;

pub trait InputLanguageNotifications {
    /// Pumps notification delivery without blocking. An error permanently
    /// selects the query fallback for this tracker instance.
    fn poll_update(&mut self) -> Result<Option<ImeState>, NotificationUnavailable>;
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

#[cfg(target_os = "macos")]
pub struct MacInputLanguageNotifications {
    monitor: diary_macos::InputLanguageMonitor,
}

#[cfg(target_os = "macos")]
impl MacInputLanguageNotifications {
    pub fn start() -> Result<Self, diary_macos::MonitorError> {
        diary_macos::InputLanguageMonitor::start().map(|monitor| Self { monitor })
    }
}

#[cfg(target_os = "macos")]
impl InputLanguageNotifications for MacInputLanguageNotifications {
    fn poll_update(&mut self) -> Result<Option<ImeState>, NotificationUnavailable> {
        self.monitor
            .poll_update()
            .map(|update| update.map(map_input_language))
            .map_err(|_| NotificationUnavailable)
    }
}

#[cfg(target_os = "macos")]
const fn map_input_language(language: diary_macos::InputLanguage) -> ImeState {
    match language {
        diary_macos::InputLanguage::English => ImeState::English,
        diary_macos::InputLanguage::Korean => ImeState::Korean,
        diary_macos::InputLanguage::Other => ImeState::Unknown,
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

/// Notification-first IME state with a permanent, deterministic transition to
/// the 500 ms polling fallback if the notification boundary cannot start or
/// later reports that delivery is unavailable.
pub struct ImeTracker<N: InputLanguageNotifications, Q: InputLanguageQuery> {
    notifications: Option<N>,
    fallback: PollingImeTracker<Q>,
    current: ImeState,
}

impl<N: InputLanguageNotifications, Q: InputLanguageQuery> ImeTracker<N, Q> {
    #[must_use]
    pub const fn new(notifications: Option<N>, fallback_query: Q) -> Self {
        Self {
            notifications,
            fallback: PollingImeTracker::new(fallback_query),
            current: ImeState::Unknown,
        }
    }

    pub fn tick(&mut self, now: Instant) -> ImeState {
        if let Some(notifications) = &mut self.notifications {
            match notifications.poll_update() {
                Ok(Some(state)) => {
                    self.current = state;
                    return state;
                }
                Ok(None) => return self.current,
                Err(NotificationUnavailable) => self.notifications = None,
            }
        }
        self.current = self.fallback.tick(now);
        self.current
    }

    #[cfg(test)]
    const fn is_using_fallback(&self) -> bool {
        self.notifications.is_none()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::VecDeque;

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

    struct FakeNotifications {
        updates: VecDeque<Result<Option<ImeState>, NotificationUnavailable>>,
        calls: u32,
    }

    impl InputLanguageNotifications for FakeNotifications {
        fn poll_update(&mut self) -> Result<Option<ImeState>, NotificationUnavailable> {
            self.calls += 1;
            self.updates.pop_front().unwrap_or(Ok(None))
        }
    }

    fn fake_query(state: ImeState) -> FakeQuery {
        FakeQuery {
            state: Cell::new(state),
            calls: Cell::new(0),
        }
    }

    #[test]
    fn the_first_tick_always_queries() {
        let fake = fake_query(ImeState::Korean);
        let mut tracker = PollingImeTracker::new(fake);
        let state = tracker.tick(Instant::now());
        assert_eq!(state, ImeState::Korean);
        assert_eq!(tracker.query.calls.get(), 1);
    }

    #[test]
    fn a_tick_before_the_interval_elapses_does_not_requery() {
        let fake = fake_query(ImeState::English);
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
        let fake = fake_query(ImeState::English);
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

    #[test]
    fn notification_updates_are_preferred_without_querying() {
        let notifications = FakeNotifications {
            updates: VecDeque::from([Ok(Some(ImeState::Korean)), Ok(None)]),
            calls: 0,
        };
        let mut tracker = ImeTracker::new(Some(notifications), fake_query(ImeState::English));
        let start = Instant::now();

        assert_eq!(tracker.tick(start), ImeState::Korean);
        assert_eq!(
            tracker.tick(start + Duration::from_millis(100)),
            ImeState::Korean
        );
        assert_eq!(tracker.fallback.query.calls.get(), 0);
        assert!(!tracker.is_using_fallback());
    }

    #[test]
    fn duplicate_notifications_keep_the_latest_state_without_querying() {
        let notifications = FakeNotifications {
            updates: VecDeque::from([
                Ok(Some(ImeState::English)),
                Ok(Some(ImeState::English)),
                Ok(Some(ImeState::Korean)),
            ]),
            calls: 0,
        };
        let mut tracker = ImeTracker::new(Some(notifications), fake_query(ImeState::Unknown));
        let start = Instant::now();

        assert_eq!(tracker.tick(start), ImeState::English);
        assert_eq!(tracker.tick(start), ImeState::English);
        assert_eq!(tracker.tick(start), ImeState::Korean);
        assert_eq!(tracker.fallback.query.calls.get(), 0);
    }

    #[test]
    fn unavailable_notification_start_uses_the_bounded_polling_fallback() {
        let mut tracker =
            ImeTracker::<FakeNotifications, _>::new(None, fake_query(ImeState::English));
        let start = Instant::now();

        assert_eq!(tracker.tick(start), ImeState::English);
        tracker.fallback.query.state.set(ImeState::Korean);
        assert_eq!(
            tracker.tick(start + Duration::from_millis(499)),
            ImeState::English
        );
        assert_eq!(tracker.fallback.query.calls.get(), 1);
        assert_eq!(
            tracker.tick(start + Duration::from_millis(500)),
            ImeState::Korean
        );
        assert_eq!(tracker.fallback.query.calls.get(), 2);
    }

    #[test]
    fn delivery_failure_switches_permanently_to_polling() {
        let notifications = FakeNotifications {
            updates: VecDeque::from([Ok(Some(ImeState::Korean)), Err(NotificationUnavailable)]),
            calls: 0,
        };
        let mut tracker = ImeTracker::new(Some(notifications), fake_query(ImeState::English));
        let start = Instant::now();

        assert_eq!(tracker.tick(start), ImeState::Korean);
        assert_eq!(
            tracker.tick(start + Duration::from_millis(100)),
            ImeState::English
        );
        assert!(tracker.is_using_fallback());
        assert_eq!(tracker.fallback.query.calls.get(), 1);
        assert_eq!(
            tracker.tick(start + Duration::from_millis(200)),
            ImeState::English
        );
        assert_eq!(tracker.fallback.query.calls.get(), 1);
    }
}
