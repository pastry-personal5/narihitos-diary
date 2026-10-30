//! Bounded retry policy and content-free setup reasons.

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryAction {
    RetryAfter(Duration),
    Pending,
    InspectAmbiguousWrite,
    Blocked,
    Conflict,
    Exhausted,
}

#[derive(Clone, Debug)]
pub struct RetryPolicy {
    attempts: u8,
    maximum_attempts: u8,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            attempts: 0,
            maximum_attempts: 5,
        }
    }
}

impl RetryPolicy {
    #[must_use]
    pub fn classify(
        &mut self,
        status: Option<u16>,
        retry_after: Option<Duration>,
        is_write: bool,
    ) -> RetryAction {
        self.attempts = self.attempts.saturating_add(1);
        if self.attempts >= self.maximum_attempts {
            return RetryAction::Exhausted;
        }
        match status {
            Some(429 | 529) => match retry_after {
                Some(delay) if delay > Duration::from_secs(60) => RetryAction::Pending,
                Some(delay) => RetryAction::RetryAfter(delay),
                None => RetryAction::RetryAfter(Duration::from_secs(
                    1_u64 << u32::from(self.attempts.saturating_sub(1)),
                )),
            },
            Some(500 | 502 | 503 | 504) if is_write => RetryAction::InspectAmbiguousWrite,
            Some(500 | 502 | 503 | 504) => RetryAction::RetryAfter(Duration::from_secs(
                1_u64 << u32::from(self.attempts.saturating_sub(1)),
            )),
            Some(401 | 403 | 404) => RetryAction::Blocked,
            Some(400) => RetryAction::Blocked,
            Some(409) => RetryAction::Conflict,
            None if is_write => RetryAction::InspectAmbiguousWrite,
            None => RetryAction::RetryAfter(Duration::from_secs(
                1_u64 << u32::from(self.attempts.saturating_sub(1)),
            )),
            Some(_) => RetryAction::Blocked,
        }
    }
}

/// Stable, content-free reason tokens accepted at connector boundaries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetupReason {
    CredentialInvalid,
    PermissionDenied,
    RootUnverifiable,
    HierarchyAmbiguous,
    RemoteUnavailable,
    ResponseInvalid,
}

#[must_use]
pub fn content_free_reason(status: Option<u16>) -> SetupReason {
    match status {
        Some(401) => SetupReason::CredentialInvalid,
        Some(403) => SetupReason::PermissionDenied,
        Some(404) => SetupReason::RootUnverifiable,
        Some(400 | 409) => SetupReason::ResponseInvalid,
        Some(_) | None => SetupReason::RemoteUnavailable,
    }
}

impl SetupReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CredentialInvalid => "notion-credential-invalid",
            Self::PermissionDenied => "notion-permission-denied",
            Self::RootUnverifiable => "notion-root-unverifiable",
            Self::HierarchyAmbiguous => "notion-hierarchy-ambiguous",
            Self::RemoteUnavailable => "notion-remote-unavailable",
            Self::ResponseInvalid => "notion-response-invalid",
        }
    }
}
